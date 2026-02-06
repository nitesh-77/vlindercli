//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - WasmRuntime: queue-based WASM agent execution
//! - ContainerRuntime: queue-based OCI container agent execution

mod container;
mod wasm;
mod wasm_plugin;

pub use container::ContainerRuntime;
pub use wasm::WasmRuntime;
