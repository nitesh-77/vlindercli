//! Agent execution engines.
//!
//! Executors run agent code in isolated environments.
//! WASM is the current implementation; Podman and Firecracker are planned.

mod wasm;

use std::sync::Arc;

use crate::domain::{Agent, InferenceEngine, Model};
use crate::embedding::{open_embedding_engine, EmbeddingEngine};
use crate::inference::open_inference_engine;
use crate::storage::{ObjectStorage, VectorStorage};

pub use wasm::WasmExecutor;

// ============================================================================
// Trait
// ============================================================================

pub trait Executor: Send + Sync {
    fn execute(
        &self,
        agent: &Agent,
        input: &str,
        object_storage: Arc<dyn ObjectStorage>,
        vector_storage: Arc<dyn VectorStorage>,
    ) -> Result<String, String>;
}

// ============================================================================
// Factory Types
// ============================================================================

/// Factory function for creating inference engines from models.
pub type InferenceFactory = Arc<dyn Fn(&Model) -> Result<Arc<dyn InferenceEngine>, String> + Send + Sync>;

/// Factory function for creating embedding engines from models.
pub type EmbeddingFactory = Arc<dyn Fn(&Model) -> Result<Arc<dyn EmbeddingEngine>, String> + Send + Sync>;

// ============================================================================
// Dispatcher
// ============================================================================

/// Create an executor appropriate for the agent's code type.
///
/// Dispatches based on the agent's code URI:
/// - `file://...*.wasm` → WasmExecutor
/// - Future: `container://` → PodmanExecutor, etc.
pub fn open_executor(agent: &Agent) -> Result<Box<dyn Executor>, String> {
    let code = &agent.code;

    if code.ends_with(".wasm") {
        Ok(Box::new(WasmExecutor::new(open_inference_engine, open_embedding_engine)))
    } else {
        Err(format!("unsupported code type: {}", code))
    }
}
