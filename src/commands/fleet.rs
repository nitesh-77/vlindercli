use std::path::PathBuf;

use clap::Subcommand;

use vlindercli::config::conversations_dir;
use vlindercli::domain::{CliHarness, ConversationStore, Daemon, Fleet, Harness};
use vlindercli::queue::agent_routing_key;

use super::repl;

#[derive(Subcommand, Debug, PartialEq)]
pub enum FleetCommand {
    /// Run a fleet interactively
    Run {
        /// Path to fleet directory (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
}

pub fn execute(cmd: FleetCommand) {
    match cmd {
        FleetCommand::Run { path } => run(path),
    }
}

pub fn run(path: Option<PathBuf>) {
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

    // Create daemon (includes runtime, provider, registry)
    let mut daemon = Daemon::new();

    // Deploy ALL agents in the fleet
    for (name, agent_path) in fleet.agents() {
        match daemon.harness.deploy_from_path(agent_path) {
            Ok(id) => {
                tracing::info!(agent = %name, id = %id, "Fleet agent deployed");
            }
            Err(e) => {
                eprintln!("Failed to deploy fleet agent '{}': {}", name, e);
                std::process::exit(1);
            }
        }
    }

    // Start session for the entry agent
    let entry_agent_id = daemon.harness.deploy_from_path(
        fleet.agent_path(&fleet.entry).expect("entry agent must exist in fleet"),
    ).unwrap_or_else(|e| {
        // deploy_from_path is idempotent for same config, this gets us the ID
        eprintln!("Failed to get entry agent ID: {}", e);
        std::process::exit(1);
    });

    let entry_agent_name = agent_routing_key(&entry_agent_id);
    daemon.harness.start_session(&entry_agent_name, conversations_dir())
        .expect("Failed to start session");

    // Read state from the system timeline (current branch)
    apply_latest_state(&mut daemon.harness, &entry_agent_name);

    println!("Fleet '{}' ready. Entry agent: {}", fleet.name, fleet.entry);

    // REPL loop: prepend fleet context to every user input, invoke entry agent
    repl::run(|input| {
        let enriched_input = format!("{}\n\n{}", fleet_context, input);
        match daemon.harness.invoke(&entry_agent_id, &enriched_input) {
            Ok(job_id) => {
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
