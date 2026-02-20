use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;

use vlindercli::config::Config;
use vlindercli::domain::{DagStore, Fleet, Harness, Registry, agent_routing_key, MessageQueue};
use vlindercli::harness::{CoreHarness, read_latest_state};
use vlindercli::queue_factory;
use vlindercli::registry_service::{GrpcRegistryClient, ping_registry};
use vlindercli::state_service::GrpcStateClient;

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

    // Connect to queue with synchronous DAG recording
    let queue: Arc<dyn MessageQueue + Send + Sync> =
        queue_factory::recording_from_config()
            .expect("Failed to create queue");

    // Create harness with remote backends (no daemon, no workers)
    let mut harness = CoreHarness::new(queue, registry);

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
    harness.start_session(&entry_agent_name);

    tracing::debug!(fleet = %fleet.name, entry = %fleet.entry, "Fleet deployed to distributed daemon");

    // Read state from the state service (ADR 079)
    apply_latest_state(&config, &mut harness, &entry_agent_name);

    println!("Fleet '{}' ready. Entry agent: {}", fleet.name, fleet.entry);

    // Run REPL with synchronous run_agent (ADR 092)
    repl::run(|input| {
        let enriched_input = format!("{}\n\n{}", fleet_context, input);
        match harness.run_agent(&entry_agent_id, &enriched_input) {
            Ok(result) => result,
            Err(e) => format!("[error] {}", e),
        }
    });
}

/// Read the latest state for an agent from the DAG store (ADR 079).
fn apply_latest_state(config: &Config, harness: &mut CoreHarness, agent_name: &str) {
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
