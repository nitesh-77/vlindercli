//! Supervisor - process manager for distributed mode.
//!
//! The Supervisor owns worker child processes. It spawns them based on config,
//! monitors their lifecycle, and terminates them on shutdown.
//!
//! This is purely a process manager — it has no domain objects (no registry,
//! no harness, no queue). Workers are self-contained processes that connect
//! to NATS and gRPC independently.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::worker_role::WorkerRole;
use vlinder_catalog::catalog_service::ping_catalog_service;
use vlinder_harness::harness_service::ping_harness;
use vlinder_nats::secret_service::ping_secret_service;
use vlinder_sql_registry::registry_service::ping_registry;
use vlinder_sql_state::state_service::ping_state_service;

/// Process manager for distributed worker processes.
pub struct Supervisor {
    workers: Vec<Child>,
}

impl Supervisor {
    /// Spawn worker processes based on config.
    pub fn new(config: &Config) -> Self {
        let counts = &config.distributed.workers;
        let mut workers = Vec::new();

        // Secret service must start first — registry needs secrets for
        // agent identity (keys). Singleton worker.
        if let Some(child) = spawn_worker(WorkerRole::Secret) {
            workers.push(child);
        }

        {
            let secret_addr = if config.distributed.secret_addr.starts_with("http://") {
                config.distributed.secret_addr.clone()
            } else {
                format!("http://{}", config.distributed.secret_addr)
            };

            let deadline = Instant::now() + Duration::from_secs(10);
            let mut version = None;

            while Instant::now() < deadline {
                if let Some(v) = ping_secret_service(&secret_addr) {
                    version = Some(v);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            match version {
                Some((major, minor, patch)) => {
                    tracing::info!(
                        addr = %secret_addr,
                        version = %format!("{}.{}.{}", major, minor, patch),
                        "Secret service is ready"
                    );
                }
                None => {
                    tracing::warn!(addr = %secret_addr, "Secret service did not become ready within 10s — registry may fail to connect");
                }
            }
        }

        // Registry must start next — other workers connect to it.
        for _ in 0..counts.registry {
            if let Some(child) = spawn_worker(WorkerRole::Registry) {
                workers.push(child);
            }
        }

        // Wait for registry to become ready before spawning client workers.
        if counts.registry > 0 {
            let addr = if config.distributed.registry_addr.starts_with("http://") {
                config.distributed.registry_addr.clone()
            } else {
                format!("http://{}", config.distributed.registry_addr)
            };

            let deadline = Instant::now() + Duration::from_secs(10);
            let mut version = None;

            while Instant::now() < deadline {
                if let Some(v) = ping_registry(&addr) {
                    version = Some(v);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            match version {
                Some((major, minor, patch)) => {
                    tracing::info!(
                        addr = %addr,
                        version = %format!("{}.{}.{}", major, minor, patch),
                        "Registry is ready"
                    );
                }
                None => {
                    tracing::error!(addr = %addr, "Registry did not become ready within 10s");
                    for child in &mut workers {
                        let _ = child.kill();
                    }
                    panic!("Registry failed to start — aborting distributed mode");
                }
            }
        }

        // State service — singleton gRPC server for DagStore queries (ADR 079).
        // Non-fatal health check: callers handle connection failures gracefully.
        if let Some(child) = spawn_worker(WorkerRole::State) {
            workers.push(child);
        }

        {
            let state_addr = if config.distributed.state_addr.starts_with("http://") {
                config.distributed.state_addr.clone()
            } else {
                format!("http://{}", config.distributed.state_addr)
            };

            let deadline = Instant::now() + Duration::from_secs(10);
            let mut version = None;

            while Instant::now() < deadline {
                if let Some(v) = ping_state_service(&state_addr) {
                    version = Some(v);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            match version {
                Some((major, minor, patch)) => {
                    tracing::info!(
                        addr = %state_addr,
                        version = %format!("{}.{}.{}", major, minor, patch),
                        "State service is ready"
                    );
                }
                None => {
                    tracing::warn!(addr = %state_addr, "State service did not become ready within 10s — state queries will fail until it starts");
                }
            }
        }

        // Catalog service — singleton gRPC server for model catalog queries.
        // Independent of other services (talks only to external APIs).
        // Non-fatal health check: CLI falls back to direct catalog access.
        if let Some(child) = spawn_worker(WorkerRole::Catalog) {
            workers.push(child);
        }

        {
            let catalog_addr = if config.distributed.catalog_addr.starts_with("http://") {
                config.distributed.catalog_addr.clone()
            } else {
                format!("http://{}", config.distributed.catalog_addr)
            };

            let deadline = Instant::now() + Duration::from_secs(10);
            let mut version = None;

            while Instant::now() < deadline {
                if let Some(v) = ping_catalog_service(&catalog_addr) {
                    version = Some(v);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            match version {
                Some((major, minor, patch)) => {
                    tracing::info!(
                        addr = %catalog_addr,
                        version = %format!("{}.{}.{}", major, minor, patch),
                        "Catalog service is ready"
                    );
                }
                None => {
                    tracing::warn!(addr = %catalog_addr, "Catalog service did not become ready within 10s — catalog queries will fail until it starts");
                }
            }
        }

        // Harness service — gRPC bridge for CLI→daemon agent invocation.
        // Must start before agent/inference workers (they depend on harness
        // being available for agent execution).
        for _ in 0..counts.harness {
            if let Some(child) = spawn_worker(WorkerRole::Harness) {
                workers.push(child);
            }
        }

        if counts.harness > 0 {
            let harness_addr = if config.distributed.harness_addr.starts_with("http://") {
                config.distributed.harness_addr.clone()
            } else {
                format!("http://{}", config.distributed.harness_addr)
            };

            let deadline = Instant::now() + Duration::from_secs(10);
            let mut version = None;

            while Instant::now() < deadline {
                if let Some(v) = ping_harness(&harness_addr) {
                    version = Some(v);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            match version {
                Some((major, minor, patch)) => {
                    tracing::info!(
                        addr = %harness_addr,
                        version = %format!("{}.{}.{}", major, minor, patch),
                        "Harness service is ready"
                    );
                }
                None => {
                    tracing::error!(addr = %harness_addr, "Harness service did not become ready within 10s");
                    for child in &mut workers {
                        let _ = child.kill();
                    }
                    panic!("Harness failed to start — aborting distributed mode");
                }
            }
        }

        // Agent runtimes
        for _ in 0..counts.agent.container {
            if let Some(child) = spawn_worker(WorkerRole::AgentContainer) {
                workers.push(child);
            }
        }

        // Inference workers
        for _ in 0..counts.inference.ollama {
            if let Some(child) = spawn_worker(WorkerRole::InferenceOllama) {
                workers.push(child);
            }
        }
        for _ in 0..counts.inference.openrouter {
            if let Some(child) = spawn_worker(WorkerRole::InferenceOpenRouter) {
                workers.push(child);
            }
        }

        // Object storage workers
        for _ in 0..counts.storage.object.sqlite {
            if let Some(child) = spawn_worker(WorkerRole::StorageObjectSqlite) {
                workers.push(child);
            }
        }
        for _ in 0..counts.storage.object.memory {
            if let Some(child) = spawn_worker(WorkerRole::StorageObjectMemory) {
                workers.push(child);
            }
        }

        // Vector storage workers
        for _ in 0..counts.storage.vector.sqlite {
            if let Some(child) = spawn_worker(WorkerRole::StorageVectorSqlite) {
                workers.push(child);
            }
        }
        for _ in 0..counts.storage.vector.memory {
            if let Some(child) = spawn_worker(WorkerRole::StorageVectorMemory) {
                workers.push(child);
            }
        }

        // DAG git worker — singleton (single branch + HEAD lock, see ADR 078).
        // The dag-sqlite consumer was removed by ADR 080: the transactional outbox
        // records nodes synchronously via the State Service on every send.
        if let Some(child) = spawn_worker(WorkerRole::DagGit) {
            workers.push(child);
        }

        // Session viewer — local HTTP server for browsing conversation sessions.
        // No health check needed: local-only, no other services depend on it.
        if let Some(child) = spawn_worker(WorkerRole::SessionViewer) {
            workers.push(child);
        }

        tracing::info!(
            worker_count = workers.len(),
            "Supervisor started in distributed mode"
        );

        Self { workers }
    }

    /// Terminate all workers and wait for them to exit.
    pub fn shutdown(&mut self) {
        for child in &mut self.workers {
            tracing::debug!(pid = child.id(), "Terminating worker");
            let _ = child.kill();
        }

        for child in &mut self.workers {
            let _ = child.wait();
        }

        self.workers.clear();
        tracing::info!("Supervisor shutdown complete");
    }
}

/// Spawn a single worker process with the given role.
fn spawn_worker(role: WorkerRole) -> Option<Child> {
    let exe = std::env::current_exe().ok()?;

    tracing::debug!(role = %role, "Spawning worker");

    match Command::new(exe)
        .env("VLINDER_WORKER_ROLE", role.as_env_value())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => {
            tracing::info!(role = %role, pid = child.id(), "Worker spawned");
            Some(child)
        }
        Err(e) => {
            tracing::error!(role = %role, error = ?e, "Failed to spawn worker");
            None
        }
    }
}
