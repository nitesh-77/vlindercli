//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - WasmRuntime: queue-based WASM agent execution
//! - Provider: aggregates service workers

mod provider;
mod wasm;

pub use provider::Provider;
pub use wasm::WasmRuntime;
