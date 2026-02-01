//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - WasmRuntime: queue-based WASM agent execution

mod wasm;

pub use wasm::WasmRuntime;
