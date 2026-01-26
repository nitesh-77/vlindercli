use std::path::PathBuf;

use clap::Subcommand;

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

    // Canonicalize to absolute path for URI
    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    // Construct file:// URI
    let uri = format!("file://{}", absolute_path.display());

    let runtime = Runtime::new();
    repl::run(|input| runtime.execute(&uri, input));
}
