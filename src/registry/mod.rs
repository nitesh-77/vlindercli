//! Registry implementations (ADR 076).
//!
//! Domain types (Registry trait, Job, JobId, etc.) live in `crate::domain`.
//! This module contains concrete implementations:
//! - `InMemoryRegistry`: In-process with RwLock, used for tests and as read cache
//! - `PersistentRegistry`: Write-through to SQLite via SqliteRegistryRepository

mod in_memory;
mod persistent;

pub use in_memory::InMemoryRegistry;
pub use persistent::PersistentRegistry;
