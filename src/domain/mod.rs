//! Domain module - the complete Vlinder protocol specification.
//!
//! This module contains both:
//! - **Data structures**: Configuration types describing WHAT to use
//! - **Capability traits**: Abstract interfaces defining the protocol
//!
//! Infrastructure modules implement these traits; the domain module
//! is the authoritative source for the runtime's abstract protocol.

mod agent;
mod agent_manifest;
mod daemon;
mod embedding;
mod fleet;
mod fleet_manifest;
mod harness;
mod inference;
mod model;
mod provider;
mod registry;
mod runtime;
mod model_manifest;
mod path;
mod resource_id;
mod storage;
pub mod workers;

// ============================================================================
// Agent & Fleet
// ============================================================================

pub use agent::{Agent, LoadError as AgentLoadError, Mount, Prompts, Requirements};
pub use path::{AbsolutePath, AbsoluteUri};
pub use agent_manifest::AgentManifest;
pub use fleet::{Fleet, LoadError as FleetLoadError};
pub use fleet_manifest::FleetManifest;

// ============================================================================
// Storage (config + traits)
// ============================================================================

pub use storage::{
    ObjectStorage, ObjectStorageType, VectorStorage, VectorStorageType,
    SqliteConfig, Storage, StorageBackend, StorageKind,
    ObjectStorageManifest, VectorStorageManifest,
};

// ============================================================================
// Resource ID (registry key)
// ============================================================================

pub use resource_id::ResourceId;

// ============================================================================
// Inference (config + trait)
// ============================================================================

pub use inference::{
    InferenceEngine,
    Inference, InferenceBackend, InferenceKind, LlamaConfig,
};

// ============================================================================
// Embedding (config + trait)
// ============================================================================

pub use embedding::{
    EmbeddingEngine,
    Embedding, EmbeddingBackend, EmbeddingKind, NomicConfig,
};

// ============================================================================
// Model & EngineType
// ============================================================================

pub use model::{Model, ModelType, EngineType, LoadError as ModelLoadError};
pub use model_manifest::{ModelManifest, ModelTypeConfig, ModelEngineConfig};

// ============================================================================
// Provider
// ============================================================================

pub use provider::Provider;

// ============================================================================
// Runtime (trait)
// ============================================================================

pub use runtime::{Runtime, RuntimeType};

// ============================================================================
// Harness (internal to Daemon)
// ============================================================================

// Harness is now owned by Daemon, not exported directly

// ============================================================================
// Daemon & Registry
// ============================================================================

pub use daemon::Daemon;
pub use registry::{JobId, JobStatus, Registry};
