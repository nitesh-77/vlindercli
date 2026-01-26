use std::path::PathBuf;

use clap::Subcommand;

use vlindercli::domain::Agent;
use vlindercli::runtime::Runtime;

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
    let agent = Agent::load(&agent_path).expect("Failed to load agent");
    let runtime = Runtime::new();

    repl::run(|input| runtime.execute(&agent, input));
}
