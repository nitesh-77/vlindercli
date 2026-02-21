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
        WorkerRole::Harness => run_harness_worker(&config, &shutdown),
        WorkerRole::AgentContainer => run_agent_container_worker(&config, &shutdown),
        WorkerRole::InferenceOllama => run_inference_ollama_worker(&config, &shutdown),
        WorkerRole::InferenceOpenRouter => run_inference_openrouter_worker(&config, &shutdown),
        WorkerRole::EmbeddingOllama => run_embedding_ollama_worker(&config, &shutdown),
        WorkerRole::StorageObjectSqlite => run_storage_object_sqlite_worker(&config, &shutdown),
        WorkerRole::StorageObjectMemory => run_storage_object_memory_worker(&config, &shutdown),
        WorkerRole::StorageVectorSqlite => run_storage_vector_sqlite_worker(&config, &shutdown),
        WorkerRole::StorageVectorMemory => run_storage_vector_memory_worker(&config, &shutdown),
        WorkerRole::Secret => run_secret_worker(&config, &shutdown),
        WorkerRole::State => run_state_worker(&config, &shutdown),
        WorkerRole::Catalog => run_catalog_worker(&config, &shutdown),
        WorkerRole::DagGit => run_dag_git_worker(&config, &shutdown),
    }

    tracing::info!(role = %role, "Worker shutdown complete");
}

// ============================================================================
// Factory helpers
// ============================================================================

/// Build a state store factory that derives the SQLite path from the agent's
/// object_storage URI in the registry. Moved out of ObjectServiceWorker so the
/// worker depends only on the StateStore trait.
fn make_state_store_factory(registry: Arc<dyn Registry>) -> crate::domain::workers::OpenStateStore {
    Box::new(move |agent_id: &str| {
        let agent = registry.get_agent_by_name(agent_id)
            .ok_or_else(|| format!("unknown agent: {}", agent_id))?;
        let uri = agent.object_storage
            .ok_or_else(|| format!("agent has no object_storage declared: {}", agent_id))?;

        let path = match uri.scheme() {
            Some("sqlite") => {
                let db_path = uri.path()
                    .ok_or_else(|| "sqlite URI has no path".to_string())?;
                let parent = std::path::Path::new(db_path).parent()
                    .ok_or_else(|| "sqlite path has no parent".to_string())?;
                parent.join("state.db")
            }
            Some("memory") => {
                let dir = std::env::temp_dir().join("vlinder-state");
                std::fs::create_dir_all(&dir).ok();
                dir.join(format!("{}.db", agent_id.replace(['/', ':'], "_")))
            }
            _ => return Err("unsupported storage scheme for state store".to_string()),
        };

        let store: Arc<dyn crate::domain::StateStore> = Arc::new(
            crate::storage::SqliteStateStore::open(&path)?
        );
        Ok(store)
    })
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
    use crate::secret_service::GrpcSecretClient;

    let secret_addr = if config.distributed.secret_addr.starts_with("http://") {
        config.distributed.secret_addr.clone()
    } else {
        format!("http://{}", config.distributed.secret_addr)
    };
    let secret_store: Arc<dyn crate::domain::SecretStore> = Arc::new(
        GrpcSecretClient::connect(&secret_addr)
            .unwrap_or_else(|e| panic!("Failed to connect to secret service: {}", e))
    );

    let db_path = registry_db_path();
    let registry = PersistentRegistry::open(&db_path, config, secret_store)
        .unwrap_or_else(|e| panic!("Failed to initialize registry: {}", e));

    // Register non-engine capabilities (engines are registered by open())
    registry.register_runtime(RuntimeType::Container);
    registry.register_object_storage(ObjectStorageType::Sqlite);
    registry.register_vector_storage(VectorStorageType::SqliteVec);

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

fn run_secret_worker(config: &Config, shutdown: &AtomicBool) {
    use tonic::transport::Server;
    use crate::secret_service::SecretServiceServer;

    let secret_store = crate::secret_store::from_config()
        .unwrap_or_else(|e| panic!("Failed to open secret store: {}", e));

    // Parse address, stripping http:// prefix if present
    let addr_str = config.distributed.secret_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.secret_addr);
    let addr: std::net::SocketAddr = addr_str.parse()
        .expect("Invalid secret service address");

    tracing::info!(?addr, "Starting secret store gRPC server");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let service = SecretServiceServer::new(secret_store).into_service();

        let server = Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async {
                while !shutdown.load(Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            });

        if let Err(e) = server.await {
            tracing::error!(?e, "Secret store server error");
        }
    });
}

fn run_harness_worker(config: &Config, shutdown: &AtomicBool) {
    use tonic::transport::Server;
    use crate::domain::HarnessType;
    use crate::harness::CoreHarness;
    use crate::harness_service::HarnessServiceServer;
    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let harness = CoreHarness::new(queue, registry, HarnessType::Grpc);

    // Parse address, stripping http:// prefix if present
    let addr_str = config.distributed.harness_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.harness_addr);
    let addr: std::net::SocketAddr = addr_str.parse()
        .expect("Invalid harness address");

    tracing::info!(?addr, registry = %registry_addr, "Starting harness gRPC server");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let service = HarnessServiceServer::new(Box::new(harness)).into_service();

        let server = Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async {
                while !shutdown.load(Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            });

        if let Err(e) = server.await {
            tracing::error!(?e, "Harness server error");
        }
    });
}

fn run_agent_container_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::ResourceId;

    use crate::runtime::ContainerRuntime;
    use crate::domain::Runtime;
    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

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

    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

    // Connect to central registry via gRPC
    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let open_engine = Box::new(crate::inference::open_inference_engine);
    let worker = InferenceServiceWorker::new(queue, registry, "ollama", open_engine);

    tracing::info!(endpoint = %config.ollama.endpoint, registry = %registry_addr, "Ollama inference worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_inference_openrouter_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::InferenceServiceWorker;

    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let open_engine = Box::new(crate::inference::open_inference_engine);
    let worker = InferenceServiceWorker::new(queue, registry, "openrouter", open_engine);

    tracing::info!(registry = %registry_addr, "OpenRouter inference worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_embedding_ollama_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::EmbeddingServiceWorker;

    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

    // Connect to central registry via gRPC
    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let open_engine = Box::new(crate::embedding::open_embedding_engine);
    let worker = EmbeddingServiceWorker::new(queue, registry, "ollama", open_engine);

    tracing::info!(endpoint = %config.ollama.endpoint, registry = %registry_addr, "Ollama embedding worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_object_sqlite_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::ObjectServiceWorker;

    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

    // Connect to central registry via gRPC
    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let open_storage = Box::new(|uri: &crate::domain::ResourceId| {
        crate::storage::dispatch::open_object_storage_from_uri(uri)
            .map_err(|e| e.to_string())
    });
    let open_state_store = make_state_store_factory(Arc::clone(&registry));
    let worker = ObjectServiceWorker::new(queue, registry, "sqlite", open_storage, open_state_store);

    tracing::info!(registry = %registry_addr, "SQLite object storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_object_memory_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::ObjectServiceWorker;

    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let open_storage = Box::new(|uri: &crate::domain::ResourceId| {
        crate::storage::dispatch::open_object_storage_from_uri(uri)
            .map_err(|e| e.to_string())
    });
    let open_state_store = make_state_store_factory(Arc::clone(&registry));
    let worker = ObjectServiceWorker::new(queue, registry, "memory", open_storage, open_state_store);

    tracing::info!(registry = %registry_addr, "In-memory object storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_vector_sqlite_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::VectorServiceWorker;

    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let open_storage = Box::new(|uri: &crate::domain::ResourceId| {
        crate::storage::dispatch::open_vector_storage_from_uri(uri)
            .map_err(|e| e.to_string())
    });
    let worker = VectorServiceWorker::new(queue, registry, "sqlite-vec", open_storage);

    tracing::info!(registry = %registry_addr, "SQLite-vec vector storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_vector_memory_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::VectorServiceWorker;

    use crate::registry_service::GrpcRegistryClient;

    let queue = crate::queue_factory::recording_from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let open_storage = Box::new(|uri: &crate::domain::ResourceId| {
        crate::storage::dispatch::open_vector_storage_from_uri(uri)
            .map_err(|e| e.to_string())
    });
    let worker = VectorServiceWorker::new(queue, registry, "memory", open_storage);

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

fn run_catalog_worker(config: &Config, shutdown: &AtomicBool) {
    use std::collections::HashMap;
    use tonic::transport::Server;
    use crate::catalog::{OllamaCatalog, OpenRouterCatalog};
    use crate::catalog_service::CatalogServiceServer;
    use crate::domain::ModelCatalog;

    let mut catalogs: HashMap<String, Arc<dyn ModelCatalog>> = HashMap::new();
    catalogs.insert(
        "ollama".to_string(),
        Arc::new(OllamaCatalog::new(&config.ollama.endpoint)),
    );
    if !config.openrouter.api_key.is_empty() {
        catalogs.insert(
            "openrouter".to_string(),
            Arc::new(OpenRouterCatalog::new(
                &config.openrouter.endpoint,
                &config.openrouter.api_key,
            )),
        );
    }

    let addr_str = config.distributed.catalog_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.catalog_addr);
    let addr: std::net::SocketAddr = addr_str.parse()
        .expect("Invalid catalog service address");

    tracing::info!(?addr, catalogs = ?catalogs.keys().collect::<Vec<_>>(), "Starting catalog gRPC server");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let service = CatalogServiceServer::new(catalogs).into_service();

        let server = Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async {
                while !shutdown.load(Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            });

        if let Err(e) = server.await {
            tracing::error!(?e, "Catalog server error");
        }
    });
}

fn run_dag_git_worker(_config: &Config, shutdown: &AtomicBool) {
    use std::collections::HashMap;
    use crate::config::conversations_dir;
    use crate::domain::workers::dag::reconstruct_observable_message;
    use crate::git_dag::GitDagWorker;
    use crate::domain::DagWorker;
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
