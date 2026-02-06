//! Daemon command - runs the vlinder daemon, supervisor, or a worker process.
//!
//! ## Usage
//!
//! Start in local mode (all services in-process):
//! ```bash
//! vlinder daemon
//! ```
//!
//! Start in distributed mode (spawns worker processes):
//! ```bash
//! # With distributed.enabled = true in config
//! vlinder daemon
//! ```
//!
//! Workers can also be started manually:
//! ```bash
//! VLINDER_WORKER_ROLE=agent-wasm vlinder daemon
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use vlindercli::config::Config;
use vlindercli::worker_role::WorkerRole;
use vlindercli::worker::run_worker_loop;
use vlindercli::domain::{Daemon, Supervisor};

/// Execute the daemon command.
///
/// Routes to one of three modes:
/// - Worker: if VLINDER_WORKER_ROLE is set
/// - Supervisor: if distributed.enabled = true
/// - Daemon: local mode (default)
pub fn execute() {
    if let Some(role) = WorkerRole::from_env() {
        run_as_worker(role);
    } else {
        let config = Config::load();
        if config.distributed.enabled {
            run_as_supervisor(&config);
        } else {
            run_as_daemon();
        }
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

/// Run as a process supervisor in distributed mode.
fn run_as_supervisor(config: &Config) {
    tracing::info!("Starting vlinder supervisor (distributed mode)");

    let mut supervisor = Supervisor::new(config);

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
    tracing::info!("Supervisor stopped");
}

/// Run as the local daemon (all services in-process).
fn run_as_daemon() {
    tracing::info!("Starting vlinder daemon (local mode)");

    let mut daemon = Daemon::new();

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal");
        shutdown_clone.store(true, Ordering::Relaxed);
    }).expect("Failed to set signal handler");

    while !shutdown.load(Ordering::Relaxed) {
        daemon.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    tracing::info!("Daemon stopped");
}
