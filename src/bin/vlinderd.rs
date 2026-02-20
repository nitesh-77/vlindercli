//! vlinderd — the Vlinder daemon process.
//!
//! Routes to one of two modes:
//! - Worker: if VLINDER_WORKER_ROLE is set
//! - Supervisor: spawns and manages worker processes

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use vlindercli::config::{conversations_dir, Config};
use vlindercli::session_server::SessionServer;
use vlindercli::supervisor::Supervisor;
use vlindercli::worker::run_worker_loop;
use vlindercli::worker_role::WorkerRole;

fn main() {
    let config = Config::load();
    vlindercli::tracing_setup::init_tracing(&config);

    if let Some(role) = WorkerRole::from_env() {
        run_as_worker(role);
    } else {
        run_as_supervisor(&config);
    }
}

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

fn run_as_supervisor(config: &Config) {
    tracing::info!("Starting vlinder supervisor (distributed mode)");

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
    tracing::info!("Supervisor stopped");
}
