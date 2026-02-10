//! Daemon command - runs the vlinder supervisor or a worker process.
//!
//! ## Usage
//!
//! Start the daemon (spawns worker processes):
//! ```bash
//! vlinder daemon
//! ```
//!
//! Workers can also be started manually:
//! ```bash
//! VLINDER_WORKER_ROLE=agent-container vlinder daemon
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use vlindercli::config::{conversations_dir, Config};
use vlindercli::worker_role::WorkerRole;
use vlindercli::worker::run_worker_loop;
use vlindercli::domain::{SessionServer, Supervisor};

/// Execute the daemon command.
///
/// Routes to one of two modes:
/// - Worker: if VLINDER_WORKER_ROLE is set
/// - Supervisor: otherwise (spawns workers)
pub fn execute() {
    if let Some(role) = WorkerRole::from_env() {
        run_as_worker(role);
    } else {
        let config = Config::load();
        run_as_supervisor(&config);
    }
}

/// Run as a worker process with the given role.
fn run_as_worker(role: WorkerRole) {
    tracing::info!(role = %role, "Starting as worker process");

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal");
        shutdown_clone.store(true, Ordering::Relaxed);
    }).expect("Failed to set signal handler");

    run_worker_loop(role, shutdown);
}

/// Run as a process supervisor — spawns and manages worker processes.
fn run_as_supervisor(config: &Config) {
    tracing::info!("Starting vlinder daemon");

    let mut supervisor = Supervisor::new(config);

    // Start session viewer
    let port = std::env::var("VLINDER_SESSION_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7777u16);
    let _server = match SessionServer::start(conversations_dir(), port) {
        Ok(server) => {
            tracing::info!(port = server.port(), "Session viewer started: http://127.0.0.1:{}", server.port());
            Some(server)
        }
        Err(e) => {
            tracing::warn!(error = %e, "Session viewer failed to start");
            None
        }
    };

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal");
        shutdown_clone.store(true, Ordering::Relaxed);
    }).expect("Failed to set signal handler");

    // Wait for shutdown signal
    while !shutdown.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    supervisor.shutdown();
    tracing::info!("Daemon stopped");
}
