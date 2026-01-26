//! Runtime Services - capabilities provided to agents.
//!
//! These are the services that the runtime makes available to agents.
//! How they are exposed (WASM host functions, HTTP, etc.) is the
//! executor's responsibility.

pub mod inference;
pub mod object_storage;
pub mod vector_storage;
