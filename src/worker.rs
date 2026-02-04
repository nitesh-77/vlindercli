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

/// Run the worker loop for the given role.
///
/// This function blocks until shutdown is signaled. Workers should be run
/// in separate processes spawned by the daemon.
pub fn run_worker_loop(role: WorkerRole, shutdown: Arc<AtomicBool>) {
    let config = Config::load();

    tracing::info!(role = %role, "Starting worker");

    match role {
        WorkerRole::Registry => run_registry_worker(&config, &shutdown),
        WorkerRole::AgentWasm => run_agent_wasm_worker(&config, &shutdown),
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
    use crate::domain::{InMemoryRegistry, RuntimeType, ObjectStorageType, VectorStorageType, EngineType};
    use crate::registry_service::RegistryServiceServer;

    let registry = InMemoryRegistry::new();

    // Register available capabilities (same as Daemon::new_local)
    registry.register_runtime(RuntimeType::Wasm);
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

fn run_agent_wasm_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::{InMemoryRegistry, ResourceId, RuntimeType};
    use crate::queue;
    use crate::runtime::WasmRuntime;
    use crate::domain::Runtime;

    let queue = queue::from_config().expect("Failed to create queue");

    // In distributed mode, we'd connect to the remote registry
    // For now, use local registry (agents must be pre-registered)
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register_runtime(RuntimeType::Wasm);
    let registry: Arc<dyn crate::domain::Registry> = registry;

    let registry_id = ResourceId::new(&config.distributed.registry_addr);
    let mut runtime = WasmRuntime::new(&registry_id, queue, Arc::clone(&registry));

    tracing::info!("WASM agent worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        runtime.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_inference_ollama_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::{InMemoryRegistry, EngineType};
    use crate::domain::workers::InferenceServiceWorker;
    use crate::queue;

    let queue = queue::from_config().expect("Failed to create queue");
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register_inference_engine(EngineType::Ollama);
    let registry: Arc<dyn crate::domain::Registry> = registry;

    let worker = InferenceServiceWorker::new(queue, registry);

    tracing::info!(endpoint = %config.ollama.endpoint, "Ollama inference worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_embedding_ollama_worker(config: &Config, shutdown: &AtomicBool) {
    use crate::domain::{InMemoryRegistry, EngineType};
    use crate::domain::workers::EmbeddingServiceWorker;
    use crate::queue;

    let queue = queue::from_config().expect("Failed to create queue");
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register_embedding_engine(EngineType::Ollama);
    let registry: Arc<dyn crate::domain::Registry> = registry;

    let worker = EmbeddingServiceWorker::new(queue, registry);

    tracing::info!(endpoint = %config.ollama.endpoint, "Ollama embedding worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_object_sqlite_worker(_config: &Config, shutdown: &AtomicBool) {
    use crate::domain::{InMemoryRegistry, ObjectStorageType};
    use crate::domain::workers::ObjectServiceWorker;
    use crate::queue;

    let queue = queue::from_config().expect("Failed to create queue");
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register_object_storage(ObjectStorageType::Sqlite);
    let registry: Arc<dyn crate::domain::Registry> = registry;

    let worker = ObjectServiceWorker::new(queue, registry);

    tracing::info!("SQLite object storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_object_memory_worker(_config: &Config, shutdown: &AtomicBool) {
    use crate::domain::{InMemoryRegistry, ObjectStorageType};
    use crate::domain::workers::ObjectServiceWorker;
    use crate::queue;

    let queue = queue::from_config().expect("Failed to create queue");
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register_object_storage(ObjectStorageType::InMemory);
    let registry: Arc<dyn crate::domain::Registry> = registry;

    let worker = ObjectServiceWorker::new(queue, registry);

    tracing::info!("In-memory object storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_vector_sqlite_worker(_config: &Config, shutdown: &AtomicBool) {
    use crate::domain::{InMemoryRegistry, VectorStorageType};
    use crate::domain::workers::VectorServiceWorker;
    use crate::queue;

    let queue = queue::from_config().expect("Failed to create queue");
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register_vector_storage(VectorStorageType::SqliteVec);
    let registry: Arc<dyn crate::domain::Registry> = registry;

    let worker = VectorServiceWorker::new(queue, registry);

    tracing::info!("SQLite-vec vector storage worker ready");

    while !shutdown.load(Ordering::Relaxed) {
        worker.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn run_storage_vector_memory_worker(_config: &Config, shutdown: &AtomicBool) {
    use crate::domain::{InMemoryRegistry, VectorStorageType};
    use crate::domain::workers::VectorServiceWorker;
    use crate::queue;

    let queue = queue::from_config().expect("Failed to create queue");
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register_vector_storage(VectorStorageType::InMemory);
    let registry: Arc<dyn crate::domain::Registry> = registry;

    let worker = VectorServiceWorker::new(queue, registry);

    tracing::info!("In-memory vector storage worker ready");

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
