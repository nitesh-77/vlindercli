use std::path::PathBuf;

use clap::Subcommand;

use vlindercli::domain::{CliHarness, Harness};

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

    // Create harness and load agent
    let mut harness = CliHarness::new();
    let agent_name = harness
        .load_agent(&absolute_path)
        .expect("Failed to load agent");

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
