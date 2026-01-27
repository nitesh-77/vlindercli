//! Agent runtime - orchestrates agent execution.

use crate::executor::open_executor;
use crate::loader;
use crate::storage::{open_object_storage, open_vector_storage};

// ============================================================================
// Runtime
// ============================================================================

pub struct Runtime;

impl Runtime {
    pub fn new() -> Self {
        Runtime
    }

    /// Execute an agent identified by URI with the provided input.
    pub fn execute(&self, uri: &str, input: &str) -> String {
        // Load agent via loader
        let agent = match loader::load_agent(uri) {
            Ok(a) => a,
            Err(e) => return format!("[error] failed to load agent: {}", e),
        };

        // Open executor, storage for this agent
        let executor = match open_executor(&agent) {
            Ok(e) => e,
            Err(e) => return format!("[error] {}", e),
        };

        let object_storage = match open_object_storage(&agent) {
            Ok(s) => s,
            Err(e) => return format!("[error] failed to open object storage: {}", e),
        };

        let vector_storage = match open_vector_storage(&agent) {
            Ok(s) => s,
            Err(e) => return format!("[error] failed to open vector storage: {}", e),
        };

        // Execute
        match executor.execute(&agent, input, object_storage, vector_storage) {
            Ok(output) => output,
            Err(e) => format!("[error] {}", e),
        }
    }
}
