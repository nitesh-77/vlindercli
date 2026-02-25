use std::path::PathBuf;

use clap::Subcommand;

use crate::config::CliConfig;
use vlinder_core::domain::{AgentManifest, Fleet, FleetManifest, Harness, agent_routing_key};

use super::connect::{connect_harness, connect_registry, open_dag_store, read_latest_state};
use super::repl;

#[derive(Subcommand, Debug, PartialEq)]
pub enum FleetCommand {
    /// Deploy a fleet manifest to the registry
    Deploy {
        /// Path to fleet directory (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Run a deployed fleet interactively
    Run {
        /// Fleet name
        name: String,
    },
    /// Create a new fleet from a template
    New {
        /// Fleet name (becomes the directory name)
        name: String,
    },
}

pub fn execute(cmd: FleetCommand) {
    match cmd {
        FleetCommand::Deploy { path } => deploy(path),
        FleetCommand::Run { name } => run(&name),
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
    println!("  vlinder fleet deploy");
    println!("  vlinder fleet run {}", name);
}

pub fn deploy(path: Option<PathBuf>) {
    let config = CliConfig::load();
    let fleet_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    let absolute_path = fleet_path
        .canonicalize()
        .expect("Failed to resolve fleet path");

    // Load fleet manifest from disk
    let manifest_path = absolute_path.join("fleet.toml");
    let manifest = FleetManifest::load(&manifest_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to load fleet manifest: {:?}", e);
            std::process::exit(1);
        });

    // Deploy all agents in the fleet via registry gRPC
    let registry = connect_registry(&config);

    for (name, agent_entry) in &manifest.agents {
        let agent_path = absolute_path.join(&agent_entry.path);
        let agent_manifest_path = agent_path.join("agent.toml");
        let agent_manifest = AgentManifest::load(&agent_manifest_path)
            .unwrap_or_else(|e| {
                eprintln!("Failed to load manifest for '{}': {:?}", name, e);
                std::process::exit(1);
            });
        let agent = registry.register_manifest(agent_manifest)
            .unwrap_or_else(|e| {
                eprintln!("Failed to deploy fleet agent '{}': {}", name, e);
                std::process::exit(1);
            });
        println!("  Agent: {} ({})", name, agent.id);
    }

    // Build Fleet from manifest + registry, then register
    let fleet = Fleet::from_manifest(manifest, &*registry)
        .unwrap_or_else(|e| {
            eprintln!("Failed to build fleet: {}", e);
            std::process::exit(1);
        });

    let fleet_name = fleet.name.clone();
    let entry_id = fleet.entry.clone();

    registry.register_fleet(fleet)
        .unwrap_or_else(|e| {
            eprintln!("Failed to register fleet: {}", e);
            std::process::exit(1);
        });

    println!("Deployed fleet '{}' (entry: {})", fleet_name, entry_id);
}

pub fn run(name: &str) {
    let config = CliConfig::load();
    let registry = connect_registry(&config);

    let fleet = match registry.get_fleet(name) {
        Some(f) => f,
        None => {
            eprintln!("Fleet '{}' not found — deploy it first with: vlinder fleet deploy", name);
            std::process::exit(1);
        }
    };

    let entry_agent_id = fleet.entry.clone();
    let entry_agent_name = agent_routing_key(&entry_agent_id);

    // Build fleet context for the entry agent
    let fleet_context = build_fleet_context(&*registry, &fleet);

    // Connect harness via gRPC — the daemon owns queue and registry
    let mut harness = connect_harness(&config);
    harness.start_session(&entry_agent_name);

    tracing::debug!(fleet = %fleet.name, "Fleet session started");

    // Read state from the state service (ADR 079)
    apply_latest_state(&config, &mut *harness, &entry_agent_name);

    println!("Fleet '{}' ready. Entry agent: {}", fleet.name, entry_agent_name);

    // Run REPL with synchronous run_agent (ADR 092)
    repl::run(|input| {
        let enriched_input = format!("{}\n\n{}", fleet_context, input);
        match harness.run_agent(&entry_agent_id, &enriched_input) {
            Ok(result) => result,
            Err(e) => format!("[error] {}", e),
        }
    });
}

/// Build a fleet context string from registered agents.
///
/// Lists all non-entry agents with their descriptions so the entry agent
/// knows what it can delegate to.
fn build_fleet_context(registry: &dyn vlinder_core::domain::Registry, fleet: &Fleet) -> String {
    let mut lines = vec![
        format!("Fleet: {}", fleet.name),
        "Available agents for delegation (use /delegate endpoint):".to_string(),
    ];

    for agent_id in &fleet.agents {
        if *agent_id == fleet.entry {
            continue;
        }
        if let Some(agent) = registry.get_agent(agent_id) {
            lines.push(format!("- {}: {}", agent.name, agent.description));
        }
    }

    lines.join("\n")
}

/// Read the latest state for an agent from the DAG store (ADR 079).
fn apply_latest_state(config: &CliConfig, harness: &mut dyn Harness, agent_name: &str) {
    let store = open_dag_store(config);
    let Some(store) = store else { return };
    if let Some(state) = read_latest_state(store.as_ref(), agent_name) {
        println!("Resuming from state {}…", &state[..8.min(state.len())]);
        harness.set_initial_state(state);
    }
}
