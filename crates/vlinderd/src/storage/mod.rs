//! Storage implementations for agents.
//!
//! Uses rusqlite (bundled) for file storage.
//! Single .db file per agent containing:
//! - files table for virtual filesystem (ObjectStorage)
//!
//! Vector storage (sqlite-vec) has moved to the vlinder-sqlite-vec provider crate.
//! Traits are defined in `crate::domain`. This module provides implementations.

pub mod dag_store;
pub mod dispatch;
mod object;
mod registry;
pub mod state_store;

// Re-export traits from domain for convenience
pub use crate::domain::ObjectStorage;
pub use crate::domain::{DagStore, DagNode, MessageType, hash_dag_node};
pub use crate::domain::{StateCommit, StateStore, hash_value, hash_snapshot, hash_state_commit};
pub use state_store::SqliteStateStore;

// Re-export concrete implementations
pub use object::SqliteObjectStorage;
pub use registry::SqliteRegistryRepository;
