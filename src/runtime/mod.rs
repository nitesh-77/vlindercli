//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - WasmRuntime: queue-based WASM agent execution
//! - Provider: aggregates service workers
//! - services: Native workers for infrastructure services

mod provider;
mod wasm;
pub mod services;

pub use provider::Provider;
pub use wasm::WasmRuntime;
pub use services::{
    ObjectServiceWorker, VectorServiceWorker,
    InferenceServiceWorker, EmbeddingServiceWorker,
};
