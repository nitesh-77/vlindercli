//! Domain module - the complete Vlinder protocol specification.
//!
//! This module contains both:
//! - **Data structures**: Configuration types describing WHAT to use
//! - **Capability traits**: Abstract interfaces defining the protocol
//!
//! Infrastructure modules implement these traits; the domain module
//! is the authoritative source for the runtime's abstract protocol.

mod agent;
mod dag;
pub mod registry_memory;
mod container_id;
mod pod_id;
mod diagnostics;
mod image_digest;
mod image_ref;
mod message;
mod message_queue;
mod agent_manifest;
mod catalog;
mod fleet;
mod fleet_manifest;
pub mod harness;
mod model;
mod provider;
mod registry;
mod registry_repository;
mod runtime;
mod session;
mod model_manifest;
mod path;
mod resource_id;
mod identity;
mod secret_store;
mod operation;
mod routing_key;
mod service_type;
mod storage;
pub mod workers;

// ============================================================================
// Agent & Fleet
// ============================================================================

pub use agent::{Agent, LoadError as AgentLoadError, Prompts, Requirements};
pub use container_id::ContainerId;
pub use pod_id::PodId;
pub use image_digest::ImageDigest;
pub use operation::Operation;
pub use service_type::ServiceType;
pub use image_ref::ImageRef;

// ============================================================================
// Message Queue (protocol types + trait)
// ============================================================================

pub use message::{
    PROTOCOL_VERSION,
    MessageId, SubmissionId, SessionId, TimelineId, Sequence, SequenceCounter, HarnessType,
    InvokeMessage, RequestMessage, ResponseMessage, CompleteMessage, DelegateMessage,
    ExpectsReply, ObservableMessage,
};
pub use message_queue::{Acknowledgement, MessageQueue, QueueError, agent_routing_key};
pub use routing_key::{RoutingKey, AgentId, Nonce, ServiceBackend, InferenceBackendType, EmbeddingBackendType};
pub use diagnostics::{
    InvokeDiagnostics, RequestDiagnostics, ServiceDiagnostics, ServiceMetrics,
    ContainerDiagnostics, ContainerRuntimeInfo, DelegateDiagnostics,
};

// ============================================================================
// DAG (content-addressed Merkle DAG)
// ============================================================================

pub use dag::{DagWorker, DagStore, DagNode, MessageType, Timeline, hash_dag_node, InMemoryDagStore};

// ============================================================================
// Paths
// ============================================================================

pub use path::{AbsolutePath, AbsoluteUri};
pub use agent_manifest::{AgentManifest, RequirementsConfig, ServiceConfig, Protocol};
pub use provider::{Provider, HttpMethod, PayloadError, TypeValidator, ProviderRoute, ProviderHost};
pub use fleet::{Fleet, LoadError as FleetLoadError};
pub use fleet_manifest::{FleetManifest, ParseError as FleetManifestParseError};

// ============================================================================
// Storage (config types only — implementations in provider crates)
// ============================================================================

pub use storage::{ObjectStorageType, VectorStorageType};

// ============================================================================
// Resource ID (registry key)
// ============================================================================

pub use resource_id::ResourceId;

// ============================================================================
// Model
// ============================================================================

pub use model::{Model, ModelType, LoadError as ModelLoadError};
pub use model_manifest::{ModelManifest, ModelTypeConfig};

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

// ============================================================================
// Secret Store (ADR 083)
// ============================================================================

pub use secret_store::{SecretStore, SecretStoreError, InMemorySecretStore};
pub use identity::{AgentIdentity, IdentityError, ensure_agent_identity};

// ============================================================================
// Registry
// ============================================================================

pub use registry::{Job, JobId, JobStatus, RegistrationError, Registry};
pub use registry_repository::{RegistryRepository, RepositoryError, StoredAgent, StoredModel};
pub use registry_memory::InMemoryRegistry;
