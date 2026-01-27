//! Storage implementations for agents.
//!
//! Uses rusqlite (bundled) + sqlite-vec for file and vector storage.
//! Single .db file per agent containing:
//! - files table for virtual filesystem (ObjectStorage)
//! - vec_items virtual table for embeddings (VectorStorage)
//!
//! Traits are defined in `crate::domain`. This module provides implementations.

pub mod dispatch;
mod object;
mod vector;

// Re-export traits from domain for convenience
pub use crate::domain::{ObjectStorage, VectorStorage};

// Re-export concrete implementations (used by external tests until commit #12)
pub use object::{InMemoryObjectStorage, SqliteObjectStorage};
pub use vector::{InMemoryVectorStorage, SqliteVectorStorage};

// Re-export factory functions
pub(crate) use object::open_object_storage;
pub(crate) use vector::open_vector_storage;
