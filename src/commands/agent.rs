use std::path::PathBuf;

use clap::Subcommand;

use vlindercli::domain::Daemon;

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

    // Create daemon
    let mut daemon = Daemon::new();

    // Deploy agent (register with Registry - runtime discovers automatically)
    let agent_id = daemon.harness.deploy_from_path(&absolute_path)
        .expect("Failed to deploy agent");

    // Run REPL
    repl::run(|input| {
        match daemon.harness.invoke(&agent_id, input) {
            Ok(job_id) => {
                // Tick until complete
                loop {
                    daemon.tick();
                    if let Some(result) = daemon.harness.poll(&job_id) {
                        return result;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            Err(e) => format!("[error] {}", e),
        }
    });
}
