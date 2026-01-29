use std::path::PathBuf;

use clap::Subcommand;

use vlindercli::domain::{Agent, CliHarness, Harness};

use super::repl;

#[derive(Subcommand, Debug, PartialEq)]
pub enum AgentCommand {
    /// Run an agent interactively
    Run {
        /// Path to agent directory (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
}

pub fn execute(cmd: AgentCommand) {
    match cmd {
        AgentCommand::Run { path } => run(path),
    }
}

fn run(path: Option<PathBuf>) {
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    // Canonicalize to absolute path
    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    // Load agent
    let agent = Agent::load(&absolute_path).expect("Failed to load agent");
    let agent_name = agent.name.clone();

    // Create harness and register agent
    let mut harness = CliHarness::new();
    harness.register(agent);

    // Run REPL
    repl::run(|input| {
        match harness.invoke(&agent_name, input) {
            Ok(request_id) => {
                match harness.run_until_response(&request_id) {
                    Ok(output) => output,
                    Err(e) => format!("[error] {}", e),
                }
            }
            Err(e) => format!("[error] {}", e),
        }
    });
}
