//! Executor dispatch - routes domain types to implementations.

use std::path::Path;
use std::sync::Arc;

use crate::domain::{ExecutorEngine, ExecutorKind, WasmConfig};
use crate::embedding::open_embedding_engine;
use crate::inference::open_inference_engine;

use super::wasm::WasmExecutor;

/// Open an executor engine for the given executor configuration.
pub(crate) fn open_executor(kind: &ExecutorKind) -> Result<Arc<dyn ExecutorEngine>, DispatchError> {
    match kind {
        ExecutorKind::Wasm(_config) => {
            Ok(Arc::new(WasmExecutor::new(open_inference_engine, open_embedding_engine)))
        }
    }
}

/// Create an ExecutorKind from an agent's code URI.
pub(crate) fn executor_kind_from_code(code: &str) -> Result<ExecutorKind, String> {
    if code.ends_with(".wasm") {
        let path = code.strip_prefix("file://").unwrap_or(code);
        Ok(ExecutorKind::Wasm(WasmConfig {
            module_path: Path::new(path).to_path_buf(),
            max_memory: 0,
        }))
    } else {
        Err(format!("unsupported code type: {}", code))
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Wasm(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Wasm(msg) => write!(f, "wasm executor error: {}", msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executor_kind_from_wasm_code() {
        let kind = executor_kind_from_code("file:///path/to/agent.wasm").unwrap();

        match kind {
            ExecutorKind::Wasm(config) => {
                assert_eq!(config.module_path.to_str().unwrap(), "/path/to/agent.wasm");
            }
        }
    }

    #[test]
    fn executor_kind_rejects_unknown_code() {
        let result = executor_kind_from_code("file:///path/to/agent.py");
        assert!(result.is_err());
    }
}
