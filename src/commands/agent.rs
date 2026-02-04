use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;

use vlindercli::config::Config;
use vlindercli::domain::{Daemon, Harness, Registry};
use vlindercli::queue::{MessageQueue, NatsQueue};
use vlindercli::registry_service::GrpcRegistryClient;

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
    let config = Config::load();

    if config.distributed.enabled {
        run_distributed(path, &config);
    } else {
        run_local(path);
    }
}

/// Run in local mode - creates embedded daemon with all services.
fn run_local(path: Option<PathBuf>) {
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    // Canonicalize to absolute path
    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    // Create daemon (includes runtime, provider, registry)
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

/// Run in distributed mode - connect as client to existing daemon.
fn run_distributed(path: Option<PathBuf>, config: &Config) {
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    tracing::info!("Connecting to distributed daemon...");

    // Connect to remote registry via gRPC
    // Ensure URL has scheme (tonic requires it)
    let registry_addr = if config.distributed.registry_addr.starts_with("http://")
        || config.distributed.registry_addr.starts_with("https://") {
        config.distributed.registry_addr.clone()
    } else {
        format!("http://{}", config.distributed.registry_addr)
    };

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
    let mut harness = Harness::new(queue, registry);

    // Deploy agent via remote registry
    let agent_id = harness.deploy_from_path(&absolute_path)
        .expect("Failed to deploy agent");

    tracing::info!(agent = %agent_id, "Agent deployed to distributed daemon");

    // Run REPL - harness ticks to process responses
    repl::run(|input| {
        match harness.invoke(&agent_id, input) {
            Ok(job_id) => {
                loop {
                    harness.tick();
                    if let Some(result) = harness.poll(&job_id) {
                        return result;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            Err(e) => format!("[error] {}", e),
        }
    });
}
