//! Storage implementations for agents.
//!
//! Object storage and state store have moved to the vlinder-sqlite-kv provider crate.
//! Vector storage has moved to the vlinder-sqlite-vec provider crate.
//! This module retains DAG storage and the registry repository.

pub mod dag_store;
mod registry;

// Re-export domain traits for convenience
pub use crate::domain::{DagStore, DagNode, MessageType, hash_dag_node};

// Re-export concrete implementations
pub use registry::SqliteRegistryRepository;
