use std::path::PathBuf;
use std::sync::Arc;

use clap::{Subcommand, ValueEnum};

use crate::config::CliConfig;
use vlinder_core::domain::{AgentManifest, Harness, Registry};

use super::connect::{connect_harness, connect_registry, open_dag_store, read_latest_state};
use super::repl;

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum Language {
    Python,
    Golang,
    Js,
    Ts,
    Java,
    Dotnet,
}

impl Language {
    fn repo_suffix(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::Golang => "golang",
            Language::Js => "js",
            Language::Ts => "ts",
            Language::Java => "java",
            Language::Dotnet => "dotnet",
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.repo_suffix())
    }
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum AgentCommand {
    /// Deploy an agent manifest to the registry
    Deploy {
        /// Path to agent directory (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Run a deployed agent interactively
    Run {
        /// Agent name
        name: String,
    },
    /// List deployed agents
    List,
    /// Show details for a deployed agent
    Get {
        /// Agent name
        name: String,
    },
    /// Create a new agent from a template
    New {
        /// Template language
        language: Language,
        /// Agent name (becomes the directory name)
        name: String,
    },
}

pub fn execute(cmd: AgentCommand) {
    match cmd {
        AgentCommand::Deploy { path } => deploy(path),
        AgentCommand::Run { name } => run(&name),
        AgentCommand::List => list(),
        AgentCommand::Get { name } => get(&name),
        AgentCommand::New { language, name } => scaffold(&language, &name),
    }
}

fn deploy(path: Option<PathBuf>) {
    let config = CliConfig::load();
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    let registry = connect_registry(&config);

    // Load manifest from disk (CLI concern) and register via registry (ADR 103)
    let manifest_path = absolute_path.join("agent.toml");
    let manifest = AgentManifest::load(&manifest_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to load agent manifest: {:?}", e);
            std::process::exit(1);
        });

    let agent = registry.register_manifest(manifest)
        .unwrap_or_else(|e| {
            eprintln!("Failed to deploy agent: {}", e);
            std::process::exit(1);
        });

    println!("Deployed: {} ({})", agent.name, agent.id);
}

fn run(name: &str) {
    let config = CliConfig::load();
    let registry = connect_registry(&config);

    // Look up already-deployed agent by name (ADR 103)
    let agent = match registry.get_agent_by_name(name) {
        Some(a) => a,
        None => {
            eprintln!("Agent '{}' not found — deploy it first with: vlinder agent deploy", name);
            std::process::exit(1);
        }
    };

    let agent_id = agent.id.clone();

    // Connect to harness via gRPC — the daemon's harness worker owns the
    // queue and registry connection. The CLI is now a pure gRPC client.
    let mut harness = connect_harness(&config);

    // Start conversation session (ADR 054, ADR 070)
    harness.start_session(name);

    // Read state from state service (ADR 079)
    apply_latest_state(&config, &mut *harness, name);

    // Run REPL with synchronous run_agent (ADR 092)
    repl::run(|input| {
        match harness.run_agent(&agent_id, input) {
            Ok(result) => result,
            Err(e) => format!("[error] {}", e),
        }
    });
}

/// Read the latest state for an agent from the DAG store (ADR 079)
/// and initialize the harness with it (state continuity across sessions).
fn apply_latest_state(config: &CliConfig, harness: &mut dyn Harness, agent_name: &str) {
    let store = open_dag_store(config);
    let Some(store) = store else { return };
    if let Some(state) = read_latest_state(store.as_ref(), agent_name) {
        println!("Resuming from state {}…", &state[..8.min(state.len())]);
        harness.set_initial_state(state);
    }
}

fn list() {
    let config = CliConfig::load();
    let registry = open_registry(&config);
    let Some(registry) = registry else { return };

    let agents = registry.get_agents();
    if agents.is_empty() {
        println!("No agents deployed.");
        return;
    }
    println!("Deployed agents:");
    for agent in agents {
        println!("  {} ({}, {})", agent.name, agent.runtime.as_str(), agent.executable);
    }
}

fn get(name: &str) {
    let config = CliConfig::load();
    let registry = open_registry(&config);
    let Some(registry) = registry else { return };

    let Some(agent) = registry.get_agent_by_name(name) else {
        eprintln!("Agent '{}' not found", name);
        return;
    };

    println!("Name:        {}", agent.name);
    println!("Runtime:     {}", agent.runtime.as_str());
    println!("Executable:  {}", agent.executable);
    println!("Description: {}", agent.description);
    if let Some(ref storage) = agent.object_storage {
        println!("Object storage: {}", storage);
    }
    if let Some(ref storage) = agent.vector_storage {
        println!("Vector storage: {}", storage);
    }
    if !agent.requirements.models.is_empty() {
        println!("Models:");
        for (alias, name) in &agent.requirements.models {
            println!("  {} -> {}", alias, name);
        }
    }
}

fn open_registry(config: &CliConfig) -> Option<Arc<dyn Registry>> {
    super::connect::open_registry(config)
}

/// Scaffold a new agent project from a GitHub template.
fn scaffold(language: &Language, name: &str) {
    let target = PathBuf::from(name);
    if target.exists() {
        eprintln!("Error: directory '{}' already exists.", name);
        std::process::exit(1);
    }

    let suffix = language.repo_suffix();
    let url = format!(
        "https://github.com/vlindercli/vlinder-agent-{}/archive/refs/heads/main.tar.gz",
        suffix
    );

    println!("Downloading {} template…", language);

    // Download tarball to a temp file
    let tarball = match download_tarball(&url) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Error: failed to download template: {}", e);
            eprintln!("Check your internet connection and try again.");
            std::process::exit(1);
        }
    };

    // Create target directory
    if let Err(e) = std::fs::create_dir_all(&target) {
        eprintln!("Error: failed to create directory '{}': {}", name, e);
        std::process::exit(1);
    }

    // Extract tarball into target directory
    if let Err(e) = extract_tarball(&tarball, &target, suffix) {
        eprintln!("Error: failed to extract template: {}", e);
        // Clean up the empty directory we created
        let _ = std::fs::remove_dir_all(&target);
        std::process::exit(1);
    }

    // Clean up temp file
    let _ = std::fs::remove_file(&tarball);

    // Patch agent name in agent.toml and build.sh
    patch_file(&target.join("agent.toml"), "hello-agent", name);
    patch_file(&target.join("build.sh"), "hello-agent", name);

    println!("Created agent '{}' from {} template.", name, language);
    println!();
    println!("Next steps:");
    println!("  cd {}", name);
    println!("  ./build.sh");
    println!("  vlinder agent deploy");
    println!("  vlinder agent run {}", name);
}

/// Download a URL to a temporary file and return the path.
fn download_tarball(url: &str) -> Result<PathBuf, String> {
    let mut resp = ureq::get(url)
        .call()
        .map_err(|e| format!("{}", e))?;

    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join("vlinder-template.tar.gz");

    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("failed to create temp file: {}", e))?;

    std::io::copy(&mut resp.body_mut().as_reader(), &mut file)
        .map_err(|e| format!("failed to write temp file: {}", e))?;

    Ok(tmp_path)
}

/// Extract a tarball, moving files from the nested directory up into target.
fn extract_tarball(tarball: &PathBuf, target: &PathBuf, suffix: &str) -> Result<(), String> {
    // tar xzf into target, then move contents from nested dir up
    let status = std::process::Command::new("tar")
        .args(["xzf", &tarball.display().to_string(), "-C", &target.display().to_string()])
        .status()
        .map_err(|e| format!("failed to run tar: {}", e))?;

    if !status.success() {
        return Err("tar extraction failed".to_string());
    }

    // GitHub archives extract to vlinder-agent-{suffix}-main/
    let nested = target.join(format!("vlinder-agent-{}-main", suffix));
    if nested.exists() {
        // Move all files from nested dir up into target
        let entries = std::fs::read_dir(&nested)
            .map_err(|e| format!("failed to read extracted directory: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("failed to read entry: {}", e))?;
            let dest = target.join(entry.file_name());
            std::fs::rename(entry.path(), dest)
                .map_err(|e| format!("failed to move file: {}", e))?;
        }

        std::fs::remove_dir_all(&nested)
            .map_err(|e| format!("failed to clean up nested directory: {}", e))?;
    }

    Ok(())
}

/// Replace all occurrences of `from` with `to` in a file.
fn patch_file(path: &PathBuf, from: &str, to: &str) {
    if let Ok(content) = std::fs::read_to_string(path) {
        let patched = content.replace(from, to);
        let _ = std::fs::write(path, patched);
    }
}
