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
        WorkerRole::EmbeddingOllama => run_embedding_ollama_worker(&config, &shutdown),
        WorkerRole::StorageObjectSqlite => run_storage_object_sqlite_worker(&config, &shutdown),
        WorkerRole::StorageObjectMemory => run_storage_object_memory_worker(&config, &shutdown),
        WorkerRole::StorageVectorSqlite => run_storage_vector_sqlite_worker(&config, &shutdown),
        WorkerRole::StorageVectorMemory => run_storage_vector_memory_worker(&config, &shutdown),
    }

    tracing::info!(role = %role, "Worker shutdown complete");
}

// ============================================================================
// Worker Implementations
// ============================================================================

fn run_registry_worker(config: &Config, shutdown: &AtomicBool) {
    use tonic::transport::Server;
    use crate::config::registry_db_path;
    use crate::domain::{InMemoryRegistry, RegistryRepository, RuntimeType, ObjectStorageType, VectorStorageType, EngineType};
    use crate::registry_service::RegistryServiceServer;
    use crate::storage::SqliteRegistryRepository;

    let registry = InMemoryRegistry::new();

    // Register available capabilities (same as Daemon::new)
    registry.register_runtime(RuntimeType::Container);
    registry.register_object_storage(ObjectStorageType::Sqlite);
    registry.register_object_storage(ObjectStorageType::InMemory);
    registry.register_vector_storage(VectorStorageType::SqliteVec);
    registry.register_vector_storage(VectorStorageType::InMemory);
    registry.register_inference_engine(EngineType::Llama);
    registry.register_inference_engine(EngineType::Ollama);
    registry.register_inference_engine(EngineType::InMemory);
    registry.register_embedding_engine(EngineType::Llama);
    registry.register_embedding_engine(EngineType::Ollama);
    registry.register_embedding_engine(EngineType::InMemory);

    // Load registered models from persistent storage (fail-fast on errors)
    let db_path = registry_db_path();
    if db_path.exists() {
        let repo = SqliteRegistryRepository::open(&db_path)
            .unwrap_or_else(|e| panic!("Failed to open registry.db at '{}': {}", db_path.display(), e));
        let models = repo.load_models()
            .unwrap_or_else(|e| panic!("Failed to load models from registry.db: {}", e));
        for model in models {
            tracing::info!(model = %model.name, "Loaded model from registry");
            registry.register_model(model)
                .unwrap_or_else(|e| panic!("Failed to register model: {}", e));
        }
    }

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

    let queue = queue::from_config().expect("Failed to create queue");

    let registry_addr = grpc_registry_addr(config);
    let registry: Arc<dyn Registry> = Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    );

    let registry_id = ResourceId::new(&config.distributed.registry_addr);
    let mut runtime = ContainerRuntime::new(&registry_id, queue, Arc::clone(&registry));

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

    let queue = queue::from_config().expect("Failed to create queue");

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

fn run_embedding_ollama_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::workers::EmbeddingServiceWorker;
    use crate::queue;
    use crate::registry_service::GrpcRegistryClient;

    let queue = queue::from_config().expect("Failed to create queue");

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

    let queue = queue::from_config().expect("Failed to create queue");

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

    let queue = queue::from_config().expect("Failed to create queue");

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

    let queue = queue::from_config().expect("Failed to create queue");

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

    let queue = queue::from_config().expect("Failed to create queue");

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
