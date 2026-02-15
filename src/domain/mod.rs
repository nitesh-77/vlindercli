//! Domain module - the complete Vlinder protocol specification.
//!
//! This module contains both:
//! - **Data structures**: Configuration types describing WHAT to use
//! - **Capability traits**: Abstract interfaces defining the protocol
//!
//! Infrastructure modules implement these traits; the domain module
//! is the authoritative source for the runtime's abstract protocol.

mod agent;
mod sdk;
mod queue_bridge;
mod dag;
mod state;
mod container_id;
mod diagnostics;
mod image_digest;
mod image_ref;
mod message;
mod message_queue;
mod agent_manifest;
mod catalog;
pub(crate) mod git_hash;
mod embedding;
mod fleet;
mod fleet_manifest;
pub mod harness;
mod inference;
mod model;
mod registry;
mod registry_repository;
mod runtime;
mod session;
mod model_manifest;
mod path;
mod resource_id;
mod route;
mod identity;
mod secret_store;
mod storage;
pub mod service_payloads;
pub mod workers;

// ============================================================================
// Agent & Fleet
// ============================================================================

pub use agent::{Agent, LoadError as AgentLoadError, Mount, Prompts, Requirements};
pub use container_id::ContainerId;
pub use image_digest::ImageDigest;
pub use image_ref::ImageRef;
pub use sdk::{SdkContract, AgentAction, AgentEvent, VectorMatch};
pub use queue_bridge::QueueBridge;

// ============================================================================
// Message Queue (protocol types + trait)
// ============================================================================

pub use message::{
    PROTOCOL_VERSION,
    MessageId, SubmissionId, SessionId, Sequence, SequenceCounter, HarnessType,
    InvokeMessage, RequestMessage, ResponseMessage, CompleteMessage, DelegateMessage,
    ExpectsReply, ObservableMessage,
};
pub use message_queue::{MessageQueue, QueueError, agent_routing_key};
pub use diagnostics::{
    InvokeDiagnostics, RequestDiagnostics, ServiceDiagnostics, ServiceMetrics,
    ContainerDiagnostics, ContainerRuntimeInfo, DelegateDiagnostics,
};

// ============================================================================
// DAG (content-addressed Merkle DAG)
// ============================================================================

pub use dag::{DagStore, DagNode, MessageType, hash_dag_node};

// ============================================================================
// State (versioned agent state)
// ============================================================================

pub use state::{StateCommit, hash_value, hash_snapshot, hash_state_commit};
pub(crate) use state::sorted_entries_json;

// ============================================================================
// Paths
// ============================================================================

pub use path::{AbsolutePath, AbsoluteUri};
pub use agent_manifest::AgentManifest;
pub use fleet::{Fleet, LoadError as FleetLoadError};
pub use fleet_manifest::FleetManifest;

// ============================================================================
// Storage (config + traits)
// ============================================================================

pub use storage::{
    ObjectStorage, ObjectStorageType, VectorStorage, VectorStorageType,
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
    InferenceEngine, InferenceResult,
    Inference, InferenceBackend, InferenceKind,
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
// Model Catalog (trait)
// ============================================================================

pub use catalog::{ModelCatalog, ModelInfo, CatalogError};

// ============================================================================
// Runtime (trait)
// ============================================================================

pub use runtime::{Runtime, RuntimeType};

// ============================================================================
// Harness (API surface for agent interaction)
// ============================================================================

pub use harness::Harness;
pub use session::{HistoryEntry, Session};
pub use route::{Route, Stop};

// ============================================================================
// Secret Store (ADR 083)
// ============================================================================

pub use secret_store::{SecretStore, SecretStoreError};
pub use identity::{AgentIdentity, IdentityError, ensure_agent_identity};

// ============================================================================
// Registry
// ============================================================================

pub use registry::{Job, JobId, JobStatus, RegistrationError, Registry};
pub use registry_repository::{RegistryRepository, RepositoryError, StoredAgent, StoredModel};
