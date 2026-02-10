//! Storage implementations for agents.
//!
//! Uses rusqlite (bundled) + sqlite-vec for file and vector storage.
//! Single .db file per agent containing:
//! - files table for virtual filesystem (ObjectStorage)
//! - vec_items virtual table for embeddings (VectorStorage)
//!
//! Traits are defined in `crate::domain`. This module provides implementations.

pub mod dag_store;
pub mod dispatch;
mod object;
mod registry;
pub mod state_store;
mod vector;

// Re-export traits from domain for convenience
pub use crate::domain::{ObjectStorage, VectorStorage};

// Re-export concrete implementations
pub use object::{InMemoryObjectStorage, SqliteObjectStorage};
pub use registry::SqliteRegistryRepository;
pub use vector::{InMemoryVectorStorage, SqliteVectorStorage};
