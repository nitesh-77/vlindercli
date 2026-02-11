use std::path::PathBuf;
use std::sync::Arc;

use clap::{Subcommand, ValueEnum};

use vlindercli::config::{registry_db_path, Config};
use vlindercli::domain::{CliHarness, Daemon, Harness, PersistentRegistry, Registry};
use vlindercli::domain::harness::read_latest_state;
use vlindercli::queue::{agent_routing_key, MessageQueue, NatsQueue};
use vlindercli::registry_service::{GrpcRegistryClient, ping_registry};

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

    if config.distributed.enabled {
        run_distributed(path, &config);
    } else {
        run_local(path);
    }
}

/// Run in local mode - creates embedded daemon with all services.
fn run_local(path: Option<PathBuf>) {
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    // Canonicalize to absolute path
    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    // Create daemon (includes runtime, provider, registry)
    let mut daemon = Daemon::new();

    // Deploy agent (register with Registry - runtime discovers automatically)
    let agent_id = daemon.harness.deploy_from_path(&absolute_path)
        .expect("Failed to deploy agent");

    // Start conversation session (ADR 054, ADR 070)
    let agent_name = agent_routing_key(&agent_id);
    daemon.harness.start_session(&agent_name);

    // Read state from file-based persistence (ADR 070)
    apply_latest_state(&mut daemon.harness, &agent_name);

    // Run REPL
    repl::run(|input| {
        match daemon.harness.invoke(&agent_id, input) {
            Ok(job_id) => {
                // Tick until complete
                loop {
                    daemon.tick();
                    if let Some(result) = daemon.harness.poll(&job_id) {
                        daemon.harness.record_response(&result);
                        return result;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            Err(e) => format!("[error] {}", e),
        }
    });
}

/// Run in distributed mode - connect as client to existing daemon.
fn run_distributed(path: Option<PathBuf>, config: &Config) {
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

    // Connect to NATS queue
    let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(
        NatsQueue::connect(&config.queue.nats_url)
            .expect("Failed to connect to NATS")
    );

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

    // Read state from file-based persistence (ADR 070)
    apply_latest_state(&mut harness, &agent_name);

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

/// Read the latest state for an agent from file-based persistence (ADR 070)
/// and initialize the harness with it (state continuity across sessions).
fn apply_latest_state(harness: &mut CliHarness, agent_name: &str) {
    if let Some(state) = read_latest_state(agent_name) {
        println!("Resuming from state {}…", &state[..8.min(state.len())]);
        harness.set_initial_state(state);
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

/// Connect to the registry — gRPC in distributed mode, local SQLite otherwise.
fn open_registry(config: &Config) -> Option<Arc<dyn Registry>> {
    if config.distributed.enabled {
        let registry_addr = if config.distributed.registry_addr.starts_with("http://")
            || config.distributed.registry_addr.starts_with("https://") {
            config.distributed.registry_addr.clone()
        } else {
            format!("http://{}", config.distributed.registry_addr)
        };

        if ping_registry(&registry_addr).is_none() {
            eprintln!("Cannot reach registry at {}. Is the daemon running?", registry_addr);
            return None;
        }

        match GrpcRegistryClient::connect(&registry_addr) {
            Ok(client) => Some(Arc::new(client)),
            Err(e) => {
                eprintln!("Failed to connect to registry: {}", e);
                None
            }
        }
    } else {
        let db_path = registry_db_path();
        match PersistentRegistry::open(&db_path, config) {
            Ok(r) => Some(Arc::new(r)),
            Err(e) => {
                eprintln!("Failed to open registry: {}", e);
                None
            }
        }
    }
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
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("{}", e))?;

    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join("vlinder-template.tar.gz");

    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("failed to create temp file: {}", e))?;

    std::io::copy(&mut resp.into_reader(), &mut file)
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
