//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - WasmRuntime: queue-based WASM agent execution

mod wasm;
mod wasm_plugin;

pub use wasm::WasmRuntime;
