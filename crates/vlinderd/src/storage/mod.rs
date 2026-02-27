//! Storage implementations for agents.
//!
//! DAG storage has moved to the vlinder-sql-state crate.
//! Object storage and state store have moved to the vlinder-sqlite-kv provider crate.
//! Vector storage has moved to the vlinder-sqlite-vec provider crate.
//! This module retains the registry repository.

mod registry;

pub use registry::SqliteRegistryRepository;
