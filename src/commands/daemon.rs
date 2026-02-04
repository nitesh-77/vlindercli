//! Daemon command - runs the vlinder daemon or a worker process.
//!
//! ## Usage
//!
//! Start the daemon (local or distributed mode based on config):
//! ```bash
//! vlinder daemon
//! ```
//!
//! In distributed mode, the daemon spawns worker processes with
//! VLINDER_WORKER_ROLE set. Workers can also be started manually:
//! ```bash
//! VLINDER_WORKER_ROLE=agent-wasm vlinder daemon
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use vlindercli::worker_role::WorkerRole;
use vlindercli::worker::run_worker_loop;
use vlindercli::domain::Daemon;

/// Execute the daemon command.
///
/// If VLINDER_WORKER_ROLE is set, runs as a worker process.
/// Otherwise, runs as the main daemon (potentially spawning workers).
pub fn execute() {
    // Check if we're a worker process
    if let Some(role) = WorkerRole::from_env() {
        run_as_worker(role);
    } else {
        run_as_daemon();
    }
}

/// Run as a worker process with the given role.
fn run_as_worker(role: WorkerRole) {
    tracing::info!(role = %role, "Starting as worker process");

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    // Set up signal handlers for graceful shutdown
    ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal");
        shutdown_clone.store(true, Ordering::Relaxed);
    }).expect("Failed to set signal handler");

    run_worker_loop(role, shutdown);
}

/// Run as the main daemon.
fn run_as_daemon() {
    tracing::info!("Starting vlinder daemon");

    let mut daemon = Daemon::new();

    if daemon.is_distributed() {
        tracing::info!("Running in distributed mode");
    } else {
        tracing::info!("Running in local mode");
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    // Set up signal handlers
    ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal");
        shutdown_clone.store(true, Ordering::Relaxed);
    }).expect("Failed to set signal handler");

    // Main tick loop
    while !shutdown.load(Ordering::Relaxed) {
        daemon.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    daemon.shutdown();
    tracing::info!("Daemon stopped");
}
