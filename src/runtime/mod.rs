//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - WasmRuntime: queue-based WASM agent execution
//! - services: Native handlers for infrastructure services

mod wasm;
pub mod services;

pub use wasm::WasmRuntime;
pub use services::{
    ObjectServiceHandler, VectorServiceHandler,
    InferenceServiceHandler, EmbeddingServiceHandler,
};
