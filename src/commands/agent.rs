use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;

use vlindercli::config::{conversations_dir, registry_db_path, Config};
use vlindercli::domain::{CliHarness, Daemon, Harness, PersistentRegistry, Registry};
use vlindercli::queue::{agent_routing_key, MessageQueue, NatsQueue};
use vlindercli::registry_service::{GrpcRegistryClient, ping_registry};

use super::repl;

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
}

pub fn execute(cmd: AgentCommand) {
    match cmd {
        AgentCommand::Run { path } => run(path),
        AgentCommand::List => list(),
        AgentCommand::Get { name } => get(&name),
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

    // Start conversation session (ADR 054)
    let agent_name = agent_routing_key(&agent_id);
    daemon.harness.start_session(&agent_name, conversations_dir())
        .expect("Failed to start session");

    // Read state from the system timeline (current branch)
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
    let agent_id = harness.deploy_from_path(&absolute_path)
        .expect("Failed to deploy agent");

    tracing::info!(agent = %agent_id, "Agent deployed to distributed daemon");

    // Start conversation session (ADR 054)
    let agent_name = agent_routing_key(&agent_id);
    harness.start_session(&agent_name, conversations_dir())
        .expect("Failed to start session");

    // Read state from the system timeline (current branch)
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

/// Read the latest state for an agent from the system timeline and initialize
/// the harness with it (state continuity across sessions).
fn apply_latest_state(harness: &mut CliHarness, agent_name: &str) {
    let store = match vlindercli::domain::ConversationStore::open(conversations_dir()) {
        Ok(s) => s,
        Err(_) => return, // No conversation store yet — first run
    };

    match store.latest_state_for_agent(agent_name) {
        Ok(Some(state)) => {
            println!("Resuming from state {}…", &state[..8.min(state.len())]);
            harness.set_initial_state(state);
        }
        Ok(None) => {} // No prior state — starting fresh
        Err(e) => {
            eprintln!("Warning: failed to read state from timeline: {}", e);
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
