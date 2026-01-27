//! Executor domain types and traits.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::agent::Agent;
use super::storage::{ObjectStorage, VectorStorage};

/// Executor engine for running agent code.
pub trait ExecutorEngine: Send + Sync {
    fn execute(
        &self,
        agent: &Agent,
        input: &str,
        object_storage: Arc<dyn ObjectStorage>,
        vector_storage: Arc<dyn VectorStorage>,
    ) -> Result<String, String>;
}

/// An executor capability for running agent code.
#[derive(Clone, Debug)]
pub struct Executor {
    pub backend: ExecutorBackend,
}

/// Backend configuration for an executor.
#[derive(Clone, Debug)]
pub struct ExecutorBackend {
    pub timeout: Duration,
    pub retries: u32,
    pub kind: ExecutorKind,
}

impl Default for ExecutorBackend {
    fn default() -> Self {
        ExecutorBackend {
            timeout: Duration::from_secs(30),
            retries: 0,
            kind: ExecutorKind::Wasm(WasmConfig::default()),
        }
    }
}

/// The specific executor implementation.
#[derive(Clone, Debug)]
pub enum ExecutorKind {
    Wasm(WasmConfig),
}

/// Configuration for WASM executor.
#[derive(Clone, Debug)]
pub struct WasmConfig {
    pub module_path: PathBuf,
    pub max_memory: u64,
}

impl Default for WasmConfig {
    fn default() -> Self {
        WasmConfig {
            module_path: PathBuf::new(),
            max_memory: 0,
        }
    }
}
