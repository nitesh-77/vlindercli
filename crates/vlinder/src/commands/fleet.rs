use std::path::PathBuf;

use clap::Subcommand;

use vlindercli::config::Config;
use vlinder_core::domain::{AgentManifest, Fleet, Harness, agent_routing_key};

use super::connect::{connect_harness, connect_registry, open_dag_store, read_latest_state};
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

    // Deploy ALL agents in the fleet via registry gRPC (ADR 103)
    let registry = connect_registry(&config);
    let mut entry_agent_id = None;

    for (name, agent_path) in fleet.agents() {
        let manifest_path = agent_path.join("agent.toml");
        let manifest = AgentManifest::load(&manifest_path)
            .unwrap_or_else(|e| {
                eprintln!("Failed to load manifest for '{}': {:?}", name, e);
                std::process::exit(1);
            });
        let agent = registry.register_manifest(manifest)
            .unwrap_or_else(|e| {
                eprintln!("Failed to deploy fleet agent '{}': {}", name, e);
                std::process::exit(1);
            });
        tracing::debug!(agent = %name, id = %agent.id, "Fleet agent deployed");
        if name == fleet.entry {
            entry_agent_id = Some(agent.id.clone());
        }
    }

    let entry_agent_id = entry_agent_id.unwrap_or_else(|| {
        eprintln!("Entry agent '{}' not found in fleet agents", fleet.entry);
        std::process::exit(1);
    });
    let entry_agent_name = agent_routing_key(&entry_agent_id);

    // Connect harness via gRPC — the daemon owns queue and registry
    let mut harness = connect_harness(&config);
    harness.start_session(&entry_agent_name);

    tracing::debug!(fleet = %fleet.name, entry = %fleet.entry, "Fleet deployed to distributed daemon");

    // Read state from the state service (ADR 079)
    apply_latest_state(&config, &mut *harness, &entry_agent_name);

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
fn apply_latest_state(config: &Config, harness: &mut dyn Harness, agent_name: &str) {
    let store = open_dag_store(config);
    let Some(store) = store else { return };
    if let Some(state) = read_latest_state(store.as_ref(), agent_name) {
        println!("Resuming from state {}…", &state[..8.min(state.len())]);
        harness.set_initial_state(state);
    }
}
