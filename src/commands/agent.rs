use std::path::PathBuf;
use std::sync::Arc;

use clap::{Subcommand, ValueEnum};

use vlindercli::config::Config;
use vlindercli::domain::{DagStore, Harness, Registry, agent_routing_key, MessageQueue};
use vlindercli::harness::{CliHarness, read_latest_state};
use vlindercli::queue;
use vlindercli::registry_service::{GrpcRegistryClient, ping_registry};
use vlindercli::state_service::GrpcStateClient;

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
    /// Run an agent interactively
    Run {
        /// Path to agent directory (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
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
        AgentCommand::Run { path } => run(path),
        AgentCommand::List => list(),
        AgentCommand::Get { name } => get(&name),
        AgentCommand::New { language, name } => scaffold(&language, &name),
    }
}

fn run(path: Option<PathBuf>) {
    let config = Config::load();
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    // Ensure URL has scheme (tonic requires it)
    let registry_addr = if config.distributed.registry_addr.starts_with("http://")
        || config.distributed.registry_addr.starts_with("https://") {
        config.distributed.registry_addr.clone()
    } else {
        format!("http://{}", config.distributed.registry_addr)
    };

    // Check registry is reachable before proceeding
    if ping_registry(&registry_addr).is_none() {
        eprintln!("Cannot reach registry at {}. Is the daemon running?", registry_addr);
        std::process::exit(1);
    }

    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    // Connect to queue with synchronous DAG recording
    let queue: Arc<dyn MessageQueue + Send + Sync> =
        queue::recording_from_config()
            .expect("Failed to create queue");

    // Create harness with remote backends (no daemon, no workers)
    let mut harness = CliHarness::new(queue, registry);

    // Deploy agent via remote registry
    let agent_id = match harness.deploy_from_path(&absolute_path) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Failed to deploy agent: {}", e);
            std::process::exit(1);
        }
    };

    tracing::info!(agent = %agent_id, "Agent deployed to distributed daemon");

    // Start conversation session (ADR 054, ADR 070)
    let agent_name = agent_routing_key(&agent_id);
    harness.start_session(&agent_name);

    // Read state from state service (ADR 079)
    apply_latest_state(&config, &mut harness, &agent_name);

    // Run REPL - harness ticks to process responses
    repl::run(|input| {
        match harness.invoke(&agent_id, input) {
            Ok(job_id) => {
                loop {
                    harness.tick();
                    if let Some(result) = harness.poll(&job_id) {
                        harness.record_response(&result);
                        return result;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            Err(e) => format!("[error] {}", e),
        }
    });
}

/// Read the latest state for an agent from the DAG store (ADR 079)
/// and initialize the harness with it (state continuity across sessions).
fn apply_latest_state(config: &Config, harness: &mut CliHarness, agent_name: &str) {
    let store = open_dag_store(config);
    let Some(store) = store else { return };
    if let Some(state) = read_latest_state(store.as_ref(), agent_name) {
        println!("Resuming from state {}…", &state[..8.min(state.len())]);
        harness.set_initial_state(state);
    }
}

/// Open the appropriate DagStore: local SQLite or remote gRPC.
fn open_dag_store(config: &Config) -> Option<Box<dyn DagStore>> {
    if config.distributed.enabled {
        let state_addr = if config.distributed.state_addr.starts_with("http://")
            || config.distributed.state_addr.starts_with("https://") {
            config.distributed.state_addr.clone()
        } else {
            format!("http://{}", config.distributed.state_addr)
        };
        match GrpcStateClient::connect(&state_addr) {
            Ok(client) => Some(Box::new(client)),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to connect to state service, skipping state read");
                None
            }
        }
    } else {
        let db_path = vlindercli::config::dag_db_path();
        if !db_path.exists() {
            return None;
        }
        match vlindercli::storage::dag_store::SqliteDagStore::open(&db_path) {
            Ok(store) => Some(Box::new(store)),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to open DAG store, skipping state read");
                None
            }
        }
    }
}

fn list() {
    let config = Config::load();
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
    let config = Config::load();
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
        for (alias, uri) in &agent.requirements.models {
            println!("  {} -> {}", alias, uri);
        }
    }
}

fn open_registry(config: &Config) -> Option<Arc<dyn Registry>> {
    vlindercli::registry::open_registry(config)
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
    println!("  vlinder agent run");
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
