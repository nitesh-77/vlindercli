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
mod fleet;
mod fleet_manifest;
mod inference;
mod model;
mod model_manifest;
mod storage;

// ============================================================================
// Agent & Fleet
// ============================================================================

pub use agent::{Agent, LoadError as AgentLoadError, Mount, Prompts, Requirements};
pub use agent_manifest::AgentManifest;
pub use fleet::{Fleet, LoadError as FleetLoadError};
pub use fleet_manifest::FleetManifest;

// ============================================================================
// Storage (config + traits)
// ============================================================================

pub use storage::{
    ObjectStorage, VectorStorage,
    SqliteConfig, Storage, StorageBackend, StorageKind,
};

// ============================================================================
// Inference (config + trait)
// ============================================================================

pub use inference::{
    InferenceEngine,
    Inference, InferenceBackend, InferenceKind, LlamaConfig,
};

// ============================================================================
// Model
// ============================================================================

pub use model::{Model, ModelType, ModelEngine, LoadError as ModelLoadError};
pub use model_manifest::{ModelManifest, ModelTypeConfig, ModelEngineConfig};
