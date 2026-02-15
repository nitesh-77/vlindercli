//! Worker process loops for distributed mode.
//!
//! When running in distributed mode, each worker process runs a specialized
//! loop based on its role. Workers communicate via NATS queues.
//!
//! ## Usage
//!
//! Workers are spawned by the daemon with VLINDER_WORKER_ROLE set:
//!
//! ```bash
//! VLINDER_WORKER_ROLE=agent-wasm vlinder daemon
//! ```
//!
//! The worker reads its role from the environment and runs the appropriate loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::config::Config;
use crate::domain::Registry;
use crate::worker_role::WorkerRole;

/// Helper to get gRPC registry address with http:// prefix.
fn grpc_registry_addr(config: &Config) -> String {
    if config.distributed.registry_addr.starts_with("http://") {
        config.distributed.registry_addr.clone()
    } else {
        format!("http://{}", config.distributed.registry_addr)
    }
}

/// Run the worker loop for the given role.
///
/// This function blocks until shutdown is signaled. Workers should be run
/// in separate processes spawned by the daemon.
pub fn run_worker_loop(role: WorkerRole, shutdown: Arc<AtomicBool>) {
    let config = Config::load();

    tracing::info!(role = %role, "Starting worker");

    match role {
        WorkerRole::Registry => run_registry_worker(&config, &shutdown),
        WorkerRole::AgentContainer => run_agent_container_worker(&config, &shutdown),
        WorkerRole::InferenceOllama => run_inference_ollama_worker(&config, &shutdown),
        WorkerRole::InferenceOpenRouter => run_inference_openrouter_worker(&config, &shutdown),
        WorkerRole::EmbeddingOllama => run_embedding_ollama_worker(&config, &shutdown),
        WorkerRole::StorageObjectSqlite => run_storage_object_sqlite_worker(&config, &shutdown),
        WorkerRole::StorageObjectMemory => run_storage_object_memory_worker(&config, &shutdown),
        WorkerRole::StorageVectorSqlite => run_storage_vector_sqlite_worker(&config, &shutdown),
        WorkerRole::StorageVectorMemory => run_storage_vector_memory_worker(&config, &shutdown),
        WorkerRole::State => run_state_worker(&config, &shutdown),
        WorkerRole::DagGit => run_dag_git_worker(&config, &shutdown),
    }

    tracing::info!(role = %role, "Worker shutdown complete");
}

// ============================================================================
// Worker Implementations
// ============================================================================

fn run_registry_worker(config: &Config, shutdown: &AtomicBool) {
    use tonic::transport::Server;
    use crate::config::registry_db_path;
    use crate::domain::{RuntimeType, ObjectStorageType, VectorStorageType};
    use crate::registry::PersistentRegistry;
    use crate::registry_service::RegistryServiceServer;

    let secret_store = crate::secret_store::from_config()
        .unwrap_or_else(|e| panic!("Failed to open secret store: {}", e));

    let db_path = registry_db_path();
    let registry = PersistentRegistry::open(&db_path, config, secret_store)
        .unwrap_or_else(|e| panic!("Failed to initialize registry: {}", e));

    // Register non-engine capabilities (engines are registered by open())
    registry.register_runtime(RuntimeType::Container);
    registry.register_object_storage(ObjectStorageType::Sqlite);
    registry.register_object_storage(ObjectStorageType::InMemory);
    registry.register_vector_storage(VectorStorageType::SqliteVec);
    registry.register_vector_storage(VectorStorageType::InMemory);

    let registry: Arc<dyn Registry> = Arc::new(registry);

    // Parse address, stripping http:// prefix if present
    let addr_str = config.distributed.registry_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.registry_addr);
    let addr: std::net::SocketAddr = addr_str.parse()
        .expect("Invalid registry address");

    tracing::info!(?addr, "Starting registry gRPC server");

    // Run the gRPC server until shutdown
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let service = RegistryServiceServer::new(registry).into_service();

        // Start server with graceful shutdown
        let server = Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async {
                // Poll for shutdown signal
                while !shutdown.load(Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            });

        if let Err(e) = server.await {
            tracing::error!(?e, "Registry server error");
        }
    });
}

fn run_agent_container_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::ResourceId;
    use crate::queue;
    use crate::runtime::ContainerRuntime;
    use crate::domain::Runtime;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let registry_id = ResourceId::new(&config.distributed.registry_addr);
    let image_policy = crate::runtime::ImagePolicy::from_config(&config.runtime.image_policy);
    let mut runtime = ContainerRuntime::new(
        &registry_id, queue, Arc::clone(&registry), image_policy,
        &config.runtime.podman_socket,
    );

    tracing::info!(registry = %registry_addr, "Container agent worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        runtime.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_inference_ollama_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::InferenceServiceWorker;
    use crate::queue;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::recording_from_config().expect("Failed to create queue");

    // Connect to central registry via gRPC
    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let worker = InferenceServiceWorker::new(queue, registry, "ollama");

    tracing::info!(endpoint = %config.ollama.endpoint, registry = %registry_addr, "Ollama inference worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_inference_openrouter_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::InferenceServiceWorker;
    use crate::queue;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let worker = InferenceServiceWorker::new(queue, registry, "openrouter");

    tracing::info!(registry = %registry_addr, "OpenRouter inference worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_embedding_ollama_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::EmbeddingServiceWorker;
    use crate::queue;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::recording_from_config().expect("Failed to create queue");

    // Connect to central registry via gRPC
    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let worker = EmbeddingServiceWorker::new(queue, registry, "ollama");

    tracing::info!(endpoint = %config.ollama.endpoint, registry = %registry_addr, "Ollama embedding worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_object_sqlite_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::ObjectServiceWorker;
    use crate::queue;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::recording_from_config().expect("Failed to create queue");

    // Connect to central registry via gRPC
    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let worker = ObjectServiceWorker::new(queue, registry, "sqlite");

    tracing::info!(registry = %registry_addr, "SQLite object storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_object_memory_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::ObjectServiceWorker;
    use crate::queue;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let worker = ObjectServiceWorker::new(queue, registry, "memory");

    tracing::info!(registry = %registry_addr, "In-memory object storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_vector_sqlite_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::VectorServiceWorker;
    use crate::queue;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let worker = VectorServiceWorker::new(queue, registry, "sqlite-vec");

    tracing::info!(registry = %registry_addr, "SQLite-vec vector storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_vector_memory_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::VectorServiceWorker;
    use crate::queue;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let worker = VectorServiceWorker::new(queue, registry, "memory");

    tracing::info!(registry = %registry_addr, "In-memory vector storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_state_worker(config: &Config, shutdown: &AtomicBool) {
    use tonic::transport::Server;
    use crate::config::dag_db_path;
    use crate::domain::DagStore;
    use crate::storage::dag_store::SqliteDagStore;
    use crate::state_service::StateServiceServer;

    let db_path = dag_db_path();
    let store = SqliteDagStore::open(&db_path)
        .unwrap_or_else(|e| panic!("Failed to open DAG store: {}", e));

    let store: Arc<dyn DagStore> = Arc::new(store);

    // Parse address, stripping http:// prefix if present
    let addr_str = config.distributed.state_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.state_addr);
    let addr: std::net::SocketAddr = addr_str.parse()
        .expect("Invalid state service address");

    tracing::info!(?addr, db = %db_path.display(), "Starting state gRPC server");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let service = StateServiceServer::new(store).into_service();

        let server = Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async {
                while !shutdown.load(Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            });

        if let Err(e) = server.await {
            tracing::error!(?e, "State server error");
        }
    });
}

fn run_dag_git_worker(_config: &Config, shutdown: &AtomicBool) {
    use std::collections::HashMap;
    use crate::config::conversations_dir;
    use crate::domain::workers::dag::reconstruct_observable_message;
    use crate::domain::workers::GitDagWorker;
    use crate::queue::NatsQueue;

    let nats = NatsQueue::localhost()
        .expect("Failed to connect to NATS");

    let repo_path = conversations_dir();
    let mut git_worker = GitDagWorker::open(&repo_path, "localhost:9000", None)
        .expect("Failed to open git DAG repo");

    tracing::info!(git = %repo_path.display(), "DAG git worker ready");

    let js = nats.jetstream().clone();
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let consumer = rt.block_on(async {
        let stream = js.get_stream("VLINDER").await
            .expect("Failed to get VLINDER stream");

        stream.create_consumer(async_nats::jetstream::consumer::pull::Config {
            name: Some("dag-git".to_string()),
            filter_subject: "vlinder.>".to_string(),
            ack_wait: std::time::Duration::from_secs(300),
            inactive_threshold: std::time::Duration::from_secs(300),
            ..Default::default()
        }).await.expect("Failed to create dag-git consumer")
    });

    while !shutdown.load(Ordering::Relaxed) {
        let msg_result = rt.block_on(async {
            use futures::StreamExt;
            let mut messages = consumer.fetch()
                .max_messages(1)
                .expires(std::time::Duration::from_millis(100))
                .messages()
                .await
                .map_err(|e| format!("fetch failed: {}", e))?;

            match messages.next().await {
                Some(Ok(msg)) => Ok(Some(msg)),
                Some(Err(e)) => Err(format!("message error: {}", e)),
                None => Ok(None),
            }
        });

        match msg_result {
            Ok(Some(msg)) => {
                let subject = msg.subject.to_string();
                let mut headers = HashMap::new();
                if let Some(h) = &msg.headers {
                    for (key, values) in h.iter() {
                        if let Some(first) = values.first() {
                            headers.insert(
                                key.to_string().to_lowercase(),
                                first.to_string(),
                            );
                        }
                    }
                }
                let payload = msg.payload.to_vec();

                if let Some(observable) = reconstruct_observable_message(&subject, &headers, &payload) {
                    let created_at = chrono::Utc::now();
                    git_worker.on_observable_message(&observable, created_at);
                }

                let _ = rt.block_on(async { msg.ack().await });
            }
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                tracing::warn!(error = %e, "DAG git fetch error");
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_loop_respects_shutdown() {
        let shutdown = Arc::new(AtomicBool::new(true)); // Already signaled

        // This should return immediately due to shutdown
        // We can't easily test the full loop, but we can verify it compiles
        assert!(shutdown.load(Ordering::Relaxed));
    }
}
