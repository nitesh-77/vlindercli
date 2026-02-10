use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;

use vlindercli::config::{conversations_dir, Config};
use vlindercli::domain::{CliHarness, ConversationStore, Fleet, Harness, Registry};
use vlindercli::queue::{agent_routing_key, MessageQueue, NatsQueue};
use vlindercli::registry_service::{GrpcRegistryClient, ping_registry};

use super::repl;

#[derive(Subcommand, Debug, PartialEq)]
pub enum FleetCommand {
    /// Run a fleet interactively
    Run {
        /// Path to fleet directory (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Create a new fleet from a template
    New {
        /// Fleet name (becomes the directory name)
        name: String,
    },
}

pub fn execute(cmd: FleetCommand) {
    match cmd {
        FleetCommand::Run { path } => run(path),
        FleetCommand::New { name } => scaffold(&name),
    }
}

fn scaffold(name: &str) {
    let target = std::path::Path::new(name);

    if target.exists() {
        eprintln!("Error: directory '{}' already exists.", name);
        std::process::exit(1);
    }

    std::fs::create_dir(target).unwrap_or_else(|e| {
        eprintln!("Error: failed to create directory '{}': {}", name, e);
        std::process::exit(1);
    });

    let fleet_toml = format!(
        r#"name = "{name}"

# Entry agent — the agent that receives user input.
# entry = "coordinator"

# Add agents using: vlinder agent new <language> agents/<name>
# Then register them here:
#
# [agents.coordinator]
# path = "agents/coordinator"
#
# [agents.researcher]
# path = "agents/researcher"

# Docs: https://docs.vlinder.ai/fleets
"#
    );

    std::fs::write(target.join("fleet.toml"), fleet_toml).unwrap_or_else(|e| {
        eprintln!("Error: failed to write fleet.toml: {}", e);
        std::process::exit(1);
    });

    println!("Created fleet '{}'.", name);
    println!();
    println!("Next steps:");
    println!("  cd {}", name);
    println!("  mkdir -p agents");
    println!("  vlinder agent new <language> agents/<agent-name>");
    println!("  # repeat for each agent, then update fleet.toml");
    println!("  vlinder fleet run");
}

/// Run a fleet interactively via NATS + gRPC registry.
///
/// Requires a running daemon (`vlinder daemon`).
pub fn run(path: Option<PathBuf>) {
    let config = Config::load();
    let fleet_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    let absolute_path = fleet_path
        .canonicalize()
        .expect("Failed to resolve fleet path");

    // Load fleet definition
    let fleet = Fleet::load(&absolute_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to load fleet: {}", e);
            std::process::exit(1);
        });

    // Build fleet context for the entry agent
    let fleet_context = fleet.build_context()
        .unwrap_or_else(|e| {
            eprintln!("Failed to build fleet context: {}", e);
            std::process::exit(1);
        });

    // Connect to the remote registry via gRPC
    let registry_addr = if config.distributed.registry_addr.starts_with("http://")
        || config.distributed.registry_addr.starts_with("https://") {
        config.distributed.registry_addr.clone()
    } else {
        format!("http://{}", config.distributed.registry_addr)
    };

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

    // Deploy ALL agents in the fleet via remote registry
    for (name, agent_path) in fleet.agents() {
        match harness.deploy_from_path(agent_path) {
            Ok(id) => {
                tracing::debug!(agent = %name, id = %id, "Fleet agent deployed");
            }
            Err(e) => {
                eprintln!("Failed to deploy fleet agent '{}': {}", name, e);
                std::process::exit(1);
            }
        }
    }

    // Start session for the entry agent
    let entry_agent_id = harness.deploy_from_path(
        fleet.agent_path(&fleet.entry).expect("entry agent must exist in fleet"),
    ).unwrap_or_else(|e| {
        eprintln!("Failed to get entry agent ID: {}", e);
        std::process::exit(1);
    });

    let entry_agent_name = agent_routing_key(&entry_agent_id);
    harness.start_session(&entry_agent_name, conversations_dir())
        .expect("Failed to start session");

    tracing::debug!(fleet = %fleet.name, entry = %fleet.entry, "Fleet deployed to distributed daemon");

    // Read state from the system timeline (current branch)
    apply_latest_state(&mut harness, &entry_agent_name);

    println!("Fleet '{}' ready. Entry agent: {}", fleet.name, fleet.entry);

    // REPL loop: harness ticks to process responses (workers handle everything else)
    repl::run(|input| {
        let enriched_input = format!("{}\n\n{}", fleet_context, input);
        match harness.invoke(&entry_agent_id, &enriched_input) {
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

/// Read the latest state for an agent from the system timeline.
fn apply_latest_state(harness: &mut CliHarness, agent_name: &str) {
    let store = match ConversationStore::open(conversations_dir()) {
        Ok(s) => s,
        Err(_) => return,
    };

    match store.latest_state_for_agent(agent_name) {
        Ok(Some(state)) => {
            println!("Resuming from state {}…", &state[..8.min(state.len())]);
            harness.set_initial_state(state);
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("Warning: failed to read state from timeline: {}", e);
        }
    }
}
