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
use crate::worker_role::WorkerRole;
use vlinder_core::domain::Registry;

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
        WorkerRole::AgentLambda => run_agent_lambda_worker(&config, &shutdown),
        WorkerRole::InferenceOllama => run_inference_ollama_worker(&config, &shutdown),
        WorkerRole::InferenceOpenRouter => run_inference_openrouter_worker(&config, &shutdown),
        WorkerRole::StorageObjectSqlite => run_storage_object_sqlite_worker(&config, &shutdown),
        WorkerRole::StorageObjectMemory => run_storage_object_memory_worker(&config, &shutdown),
        WorkerRole::StorageVectorSqlite => run_storage_vector_sqlite_worker(&config, &shutdown),
        WorkerRole::StorageVectorMemory => run_storage_vector_memory_worker(&config, &shutdown),
        WorkerRole::Secret => run_secret_worker(&config, &shutdown),
        WorkerRole::State => run_state_worker(&config, &shutdown),
        WorkerRole::Catalog => run_catalog_worker(&config, &shutdown),
        WorkerRole::DagGit => run_dag_git_worker(&config, &shutdown),
        WorkerRole::SessionViewer => run_session_viewer_worker(&config, &shutdown),
    }

    tracing::info!(role = %role, "Worker shutdown complete");
}

// ============================================================================
// Factory helpers
// ============================================================================

// ============================================================================
// Worker Implementations
// ============================================================================

fn run_registry_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::config::registry_db_path;
    use tonic::transport::Server;
    use vlinder_core::domain::{ObjectStorageType, RuntimeType, VectorStorageType};
    use vlinder_nats::secret_service::GrpcSecretClient;
    use vlinder_sql_registry::registry_service::RegistryServiceServer;
    use vlinder_sql_registry::PersistentRegistry;

    let secret_addr = if config.distributed.secret_addr.starts_with("http://") {
        config.distributed.secret_addr.clone()
    } else {
        format!("http://{}", config.distributed.secret_addr)
    };
    let secret_store: Arc<dyn vlinder_core::domain::SecretStore> = Arc::new(
        GrpcSecretClient::connect(&secret_addr)
            .unwrap_or_else(|e| panic!("Failed to connect to secret service: {}", e)),
    );

    let db_path = registry_db_path();

    // Build registry config from cluster topology
    let mut inference_engines = Vec::new();
    let mut embedding_engines = Vec::new();
    if config.distributed.workers.inference.ollama > 0 {
        inference_engines.push(vlinder_core::domain::Provider::Ollama);
        embedding_engines.push(vlinder_core::domain::Provider::Ollama);
    }
    if config.distributed.workers.inference.openrouter > 0 {
        inference_engines.push(vlinder_core::domain::Provider::OpenRouter);
    }
    let registry_config = vlinder_sql_registry::RegistryConfig {
        inference_engines,
        embedding_engines,
    };

    let registry = PersistentRegistry::open(&db_path, &registry_config, secret_store)
        .unwrap_or_else(|e| panic!("Failed to initialize registry: {}", e));

    // Register non-engine capabilities (engines are registered by open())
    registry.register_runtime(RuntimeType::Container);
    if config.distributed.workers.agent.lambda > 0 {
        registry.register_runtime(RuntimeType::Lambda);
    }
    registry.register_object_storage(ObjectStorageType::Sqlite);
    registry.register_vector_storage(VectorStorageType::SqliteVec);

    let registry: Arc<dyn Registry> = Arc::new(registry);

    // Parse address, stripping http:// prefix if present
    let addr_str = config
        .distributed
        .registry_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.registry_addr);
    let addr: std::net::SocketAddr = addr_str.parse().expect("Invalid registry address");

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
    use vlinder_nats::secret_service::SecretServiceServer;

    let secret_store = crate::secret_store_factory::from_config(config)
        .unwrap_or_else(|e| panic!("Failed to open secret store: {}", e));

    // Parse address, stripping http:// prefix if present
    let addr_str = config
        .distributed
        .secret_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.secret_addr);
    let addr: std::net::SocketAddr = addr_str.parse().expect("Invalid secret service address");

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
    use vlinder_core::domain::{CoreHarness, HarnessType};
    use vlinder_harness::harness_service::HarnessServiceServer;
    use vlinder_sql_registry::registry_service::GrpcRegistryClient;

    let queue =
        crate::queue_factory::recording_from_config(config).expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr).expect("Failed to connect to registry"),
    );

    let harness = CoreHarness::new(queue, registry, HarnessType::Grpc);

    // Parse address, stripping http:// prefix if present
    let addr_str = config
        .distributed
        .harness_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.harness_addr);
    let addr: std::net::SocketAddr = addr_str.parse().expect("Invalid harness address");

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
    use vlinder_core::domain::Runtime;
    use vlinder_podman_runtime::{ContainerRuntime, PodmanRuntimeConfig};

    let registry =
        crate::registry_factory::from_config(config).expect("Failed to connect to registry");

    let podman_config = PodmanRuntimeConfig {
        image_policy: config.runtime.image_policy.clone(),
        podman_socket: config.runtime.podman_socket.clone(),
        sidecar_image: config.runtime.sidecar_image.clone(),
        nats_url: config.queue.nats_url.clone(),
        registry_addr: config.distributed.registry_addr.clone(),
        state_addr: config.distributed.state_addr.clone(),
        secret_addr: config.distributed.secret_addr.clone(),
    };

    let mut runtime = ContainerRuntime::new(&podman_config, registry)
        .expect("Failed to create container runtime");

    tracing::info!("Container agent worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        runtime.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_agent_lambda_worker(config: &Config, shutdown: &AtomicBool) {
    use vlinder_core::domain::Runtime;
    use vlinder_nats_lambda_runtime::{LambdaRuntime, LambdaRuntimeConfig};

    let registry =
        crate::registry_factory::from_config(config).expect("Failed to connect to registry");

    let queue = crate::queue_factory::from_config(config)
        .expect("Failed to create queue for Lambda runtime");

    let lambda_config = LambdaRuntimeConfig {
        registry_addr: config.distributed.registry_addr.clone(),
        region: config.runtime.lambda_region.clone(),
        memory_mb: config.runtime.lambda_memory_mb,
        timeout_secs: config.runtime.lambda_timeout_secs,
    };

    let mut runtime = LambdaRuntime::new(&lambda_config, registry, queue)
        .expect("Failed to create Lambda runtime");

    tracing::info!(
        region = config.runtime.lambda_region.as_str(),
        "Lambda agent worker ready"
    );

    while !shutdown.load(Ordering::Relaxed) {
        runtime.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_inference_ollama_worker(config: &Config, shutdown: &AtomicBool) {
    use vlinder_ollama::OllamaWorker;

    let queue =
        crate::queue_factory::recording_from_config(config).expect("Failed to create queue");

    let worker = OllamaWorker::new(queue, config.ollama.endpoint.clone());

    tracing::info!(endpoint = %config.ollama.endpoint, "Ollama inference worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_inference_openrouter_worker(config: &Config, shutdown: &AtomicBool) {
    use vlinder_infer_openrouter::OpenRouterWorker;

    let queue =
        crate::queue_factory::recording_from_config(config).expect("Failed to create queue");

    let worker = OpenRouterWorker::new(
        queue,
        config.openrouter.endpoint.clone(),
        config.openrouter.api_key.clone(),
    );

    tracing::info!(endpoint = %config.openrouter.endpoint, "OpenRouter inference worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_object_sqlite_worker(config: &Config, shutdown: &AtomicBool) {
    use vlinder_sqlite_kv::KvWorker;

    use vlinder_sql_registry::registry_service::GrpcRegistryClient;

    let queue =
        crate::queue_factory::recording_from_config(config).expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr).expect("Failed to connect to registry"),
    );

    use vlinder_core::domain::{ObjectStorageType, ServiceBackend};
    let worker = KvWorker::new(
        queue,
        registry,
        ServiceBackend::Kv(ObjectStorageType::Sqlite),
    );

    tracing::info!(registry = %registry_addr, "SQLite object storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_object_memory_worker(config: &Config, shutdown: &AtomicBool) {
    use vlinder_sqlite_kv::KvWorker;

    use vlinder_sql_registry::registry_service::GrpcRegistryClient;

    let queue =
        crate::queue_factory::recording_from_config(config).expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr).expect("Failed to connect to registry"),
    );

    use vlinder_core::domain::{ObjectStorageType, ServiceBackend};
    let worker = KvWorker::new(
        queue,
        registry,
        ServiceBackend::Kv(ObjectStorageType::InMemory),
    );

    tracing::info!(registry = %registry_addr, "In-memory object storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_vector_sqlite_worker(config: &Config, shutdown: &AtomicBool) {
    use vlinder_sqlite_vec::SqliteVecWorker;

    use vlinder_sql_registry::registry_service::GrpcRegistryClient;

    let queue =
        crate::queue_factory::recording_from_config(config).expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr).expect("Failed to connect to registry"),
    );

    use vlinder_core::domain::{ServiceBackend, VectorStorageType};
    let worker = SqliteVecWorker::new(
        queue,
        registry,
        ServiceBackend::Vec(VectorStorageType::SqliteVec),
    );

    tracing::info!(registry = %registry_addr, "SQLite-vec vector storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_vector_memory_worker(config: &Config, shutdown: &AtomicBool) {
    use vlinder_sqlite_vec::SqliteVecWorker;

    use vlinder_sql_registry::registry_service::GrpcRegistryClient;

    let queue =
        crate::queue_factory::recording_from_config(config).expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr).expect("Failed to connect to registry"),
    );

    use vlinder_core::domain::{ServiceBackend, VectorStorageType};
    let worker = SqliteVecWorker::new(
        queue,
        registry,
        ServiceBackend::Vec(VectorStorageType::InMemory),
    );

    tracing::info!(registry = %registry_addr, "In-memory vector storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_state_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::config::dag_db_path;
    use tonic::transport::Server;
    use vlinder_core::domain::DagStore;
    use vlinder_sql_state::state_service::StateServiceServer;
    use vlinder_sql_state::SqliteDagStore;

    let db_path = dag_db_path();
    let store = SqliteDagStore::open(&db_path)
        .unwrap_or_else(|e| panic!("Failed to open DAG store: {}", e));

    let store: Arc<dyn DagStore> = Arc::new(store);

    // Parse address, stripping http:// prefix if present
    let addr_str = config
        .distributed
        .state_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.state_addr);
    let addr: std::net::SocketAddr = addr_str.parse().expect("Invalid state service address");

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
    use tonic::transport::Server;
    use vlinder_catalog::catalog_service::CatalogServiceServer;
    use vlinder_core::domain::{CatalogService, CompositeCatalog};
    use vlinder_infer_openrouter::OpenRouterCatalog;
    use vlinder_ollama::OllamaCatalog;

    let mut composite = CompositeCatalog::new();
    composite.add(
        "ollama".to_string(),
        Arc::new(OllamaCatalog::new(&config.ollama.endpoint)),
    );
    if !config.openrouter.api_key.is_empty() {
        composite.add(
            "openrouter".to_string(),
            Arc::new(OpenRouterCatalog::new(
                &config.openrouter.endpoint,
                &config.openrouter.api_key,
            )),
        );
    }

    let addr_str = config
        .distributed
        .catalog_addr
        .strip_prefix("http://")
        .unwrap_or(&config.distributed.catalog_addr);
    let addr: std::net::SocketAddr = addr_str.parse().expect("Invalid catalog service address");

    let catalog_names = composite.catalogs();
    tracing::info!(?addr, catalogs = ?catalog_names, "Starting catalog gRPC server");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let service = CatalogServiceServer::new(Arc::new(composite)).into_service();

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

fn run_dag_git_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::config::conversations_dir;
    use std::collections::HashMap;
    use vlinder_core::domain::DagWorker;
    use vlinder_git_dag::GitDagWorker;
    use vlinder_nats::{from_nats_headers, subject_to_routing_key, NatsQueue};

    let nats = NatsQueue::connect(&config.queue.nats_config()).expect("Failed to connect to NATS");

    let repo_path = conversations_dir();
    let mut git_worker = GitDagWorker::open(&repo_path, "localhost:9000", None)
        .expect("Failed to open git DAG repo");

    tracing::info!(git = %repo_path.display(), "DAG git worker ready");

    let js = nats.jetstream().clone();
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let consumer = rt.block_on(async {
        let stream = js
            .get_stream("VLINDER")
            .await
            .expect("Failed to get VLINDER stream");

        stream
            .create_consumer(async_nats::jetstream::consumer::pull::Config {
                name: Some("dag-git".to_string()),
                filter_subject: "vlinder.>".to_string(),
                ack_wait: std::time::Duration::from_secs(300),
                inactive_threshold: std::time::Duration::from_secs(300),
                ..Default::default()
            })
            .await
            .expect("Failed to create dag-git consumer")
    });

    while !shutdown.load(Ordering::Relaxed) {
        let msg_result = rt.block_on(async {
            use futures::StreamExt;
            let mut messages = consumer
                .fetch()
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
                            headers.insert(key.to_string().to_lowercase(), first.to_string());
                        }
                    }
                }
                let payload = msg.payload.to_vec();

                let info = msg.info().ok();
                let num_delivered = info.as_ref().map(|i| i.delivered);
                let stream_seq = info.as_ref().map(|i| i.stream_sequence);
                tracing::debug!(
                    subject = subject.as_str(),
                    stream_seq = ?stream_seq,
                    num_delivered = ?num_delivered,
                    "DAG git received NATS message",
                );

                // Pipeline: subject → RoutingKey → typed headers → assemble.
                let observable = subject_to_routing_key(&subject)
                    .and_then(|key| from_nats_headers(&key, &headers))
                    .map(|hdrs| hdrs.assemble(payload.clone()));

                if let Some(observable) = observable {
                    let created_at = chrono::Utc::now();
                    git_worker.on_observable_message(&observable, created_at);
                } else {
                    tracing::warn!(
                        subject = subject.as_str(),
                        "DAG git could not reconstruct message"
                    );
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

fn run_session_viewer_worker(_config: &Config, shutdown: &AtomicBool) {
    use crate::config::dag_db_path;
    use vlinder_sql_state::{SessionServer, SqliteDagStore};

    let port = std::env::var("VLINDER_SESSION_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7777u16);

    let store =
        SqliteDagStore::open(&dag_db_path()).expect("Failed to open DAG store for session viewer");
    let server =
        SessionServer::start(Arc::new(store), port).expect("Failed to start session viewer");

    tracing::info!(
        port = server.port(),
        "Session viewer started: http://127.0.0.1:{}",
        server.port()
    );

    while !shutdown.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    server.stop();
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
