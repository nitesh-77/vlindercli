//! Agent execution engines.
//!
//! The trait is defined in the domain module.

mod wasm;

use std::sync::Arc;

use crate::domain::{Agent, EmbeddingEngine, ExecutorEngine, InferenceEngine, Model};
use crate::embedding::open_embedding_engine;
use crate::inference::open_inference_engine;

pub use wasm::WasmExecutor;

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
pub fn open_executor(agent: &Agent) -> Result<Box<dyn ExecutorEngine>, String> {
    let code = &agent.code;

    if code.ends_with(".wasm") {
        Ok(Box::new(WasmExecutor::new(open_inference_engine, open_embedding_engine)))
    } else {
        Err(format!("unsupported code type: {}", code))
    }
}
