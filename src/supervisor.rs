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
use crate::registry_service::ping_registry;
use crate::worker_role::WorkerRole;

/// Process manager for distributed worker processes.
pub struct Supervisor {
    workers: Vec<Child>,
}

impl Supervisor {
    /// Spawn worker processes based on config.
    pub fn new(config: &Config) -> Self {
        let counts = &config.distributed.workers;
        let mut workers = Vec::new();

        // Registry must start first — other workers connect to it.
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

        // Embedding workers
        for _ in 0..counts.embedding.ollama {
            if let Some(child) = spawn_worker(WorkerRole::EmbeddingOllama) {
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

        // DAG workers — always exactly one of each (singleton by design, see ADR 078).
        // Git: single branch + HEAD lock. SQLite: per-session Merkle chains in one process.
        // Scaling is a future workload-driven decision (ref-per-session, NATS subject routing).
        if let Some(child) = spawn_worker(WorkerRole::DagSqlite) {
            workers.push(child);
        }
        if let Some(child) = spawn_worker(WorkerRole::DagGit) {
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
        .args(["daemon"])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_with_zero_counts_spawns_nothing() {
        let config = Config {
            distributed: crate::config::DistributedConfig {
                enabled: true,
                registry_addr: "http://127.0.0.1:9090".to_string(),
                workers: crate::config::WorkerCounts {
                    registry: 0,
                    agent: crate::config::AgentWorkerCounts { container: 0 },
                    inference: crate::config::InferenceWorkerCounts { ollama: 0, openrouter: 0 },
                    embedding: crate::config::EmbeddingWorkerCounts { ollama: 0 },
                    storage: crate::config::StorageWorkerCounts {
                        object: crate::config::ObjectStorageWorkerCounts { sqlite: 0, memory: 0 },
                        vector: crate::config::VectorStorageWorkerCounts { sqlite: 0, memory: 0 },
                    },
                },
            },
            ..Default::default()
        };

        let mut supervisor = Supervisor::new(&config);
        // Only the two unconditional DAG singletons (dag-sqlite, dag-git) are spawned.
        // All config-driven workers have count 0.
        assert_eq!(supervisor.workers.len(), 2);
        supervisor.shutdown();
    }
}
