//! Registry - source of truth for all system state.
//!
//! Stores:
//! - Runtimes (available at `/runtimes`)
//! - Models (registered model definitions)
//! - Agents (registered agent definitions)
//! - Jobs (submitted, running, completed)
//!
//! The `Registry` trait abstracts state storage. Implementations live
//! outside the domain module:
//! - `InMemoryRegistry` — `crate::registry`
//! - `PersistentRegistry` — `crate::registry`
//! - `GrpcRegistryClient` — `crate::registry_service`

use crate::domain::{Agent, EngineType, Model, ObjectStorageType, ResourceId, RuntimeType, VectorStorageType};

/// Unique identifier for a submitted job.
///
/// Format: `<registry_id>/jobs/<uuid>`
/// Example: `http://127.0.0.1:9000/jobs/550e8400-e29b-41d4-a716-446655440000`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JobId(String);

impl JobId {
    /// Create a new JobId under the given registry.
    pub fn new(registry_id: &ResourceId) -> Self {
        let uuid = uuid::Uuid::new_v4();
        Self(format!("{}/jobs/{}", registry_id.as_str(), uuid))
    }

    /// Create a JobId from an existing string (e.g., from gRPC).
    pub fn from_string(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A submitted job and its current state.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: JobId,
    pub submission_id: super::SubmissionId,  // ADR 044: tracks message flow
    pub agent_id: ResourceId,
    pub input: String,
    pub status: JobStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Completed(String),
    Failed(String),
}

/// Error returned when agent registration fails validation.
#[derive(Debug)]
pub enum RegistrationError {
    /// Agent with this name is already registered.
    DuplicateName(String),
    /// No runtime available for this agent's executable scheme/extension.
    NoRuntime(ResourceId),
    /// Agent declares object storage with unknown scheme.
    UnknownObjectStorageScheme(String),
    /// Agent declares object storage type not available.
    ObjectStorageUnavailable(ObjectStorageType),
    /// Agent declares vector storage with unknown scheme.
    UnknownVectorStorageScheme(String),
    /// Agent declares vector storage type not available.
    VectorStorageUnavailable(VectorStorageType),
    /// Agent requires a model that is not registered.
    /// Contains the agent's alias and the model_path URI.
    ModelNotRegistered(String, ResourceId),
    /// Agent requires an inference engine that is not available.
    InferenceEngineUnavailable(EngineType, String),
    /// Agent requires an embedding engine that is not available.
    EmbeddingEngineUnavailable(EngineType, String),
    /// Model cannot be removed because deployed agents depend on it.
    ModelInUse(String, Vec<String>),
    /// Persistence operation failed (disk I/O, database error, etc.).
    Persistence(String),
    /// Error forwarded from a remote registry (gRPC).
    /// Contains the server's error message as-is — already user-friendly.
    Remote(String),
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistrationError::DuplicateName(name) => write!(f, "agent already registered: {}", name),
            RegistrationError::NoRuntime(id) => {
                write!(f, "no runtime available for agent executable: {}\n\nIs the daemon running? Start it with: vlinder daemon", id)
            }
            RegistrationError::UnknownObjectStorageScheme(s) => write!(f, "unknown object storage scheme: {}", s),
            RegistrationError::ObjectStorageUnavailable(t) => write!(f, "object storage not available: {:?}", t),
            RegistrationError::UnknownVectorStorageScheme(s) => write!(f, "unknown vector storage scheme: {}", s),
            RegistrationError::VectorStorageUnavailable(t) => write!(f, "vector storage not available: {:?}", t),
            RegistrationError::ModelNotRegistered(alias, uri) => {
                let hint = model_add_hint(uri.as_str());
                write!(f, "model '{}' is not registered ({})\n\nAdd it first: {}", alias, uri, hint)
            }
            RegistrationError::InferenceEngineUnavailable(engine, model) => {
                write!(f, "no {} inference engine available for model '{}'\n\nIs the daemon running? Start it with: vlinder daemon", engine.as_backend_str(), model)
            }
            RegistrationError::EmbeddingEngineUnavailable(engine, model) => {
                write!(f, "no {} embedding engine available for model '{}'\n\nIs the daemon running? Start it with: vlinder daemon", engine.as_backend_str(), model)
            }
            RegistrationError::ModelInUse(name, agents) => write!(f, "model '{}' is in use by agents: {}", name, agents.join(", ")),
            RegistrationError::Persistence(msg) => write!(f, "persistence error: {}", msg),
            RegistrationError::Remote(msg) => write!(f, "{}", msg),
        }
    }
}

/// Build a complete `vlinder model add` command from a model_path URI.
///
/// The scheme determines the catalog and how to extract the model name:
/// - `ollama://host:port/name:tag` → `vlinder model add name`
/// - `openrouter://provider/model`  → `vlinder model add provider/model --catalog openrouter`
///
/// Examples:
/// - `ollama://localhost:11434/nomic-embed-text:latest` → `"vlinder model add nomic-embed-text"`
/// - `openrouter://anthropic/claude-sonnet-4` → `"vlinder model add anthropic/claude-sonnet-4 --catalog openrouter"`
fn model_add_hint(uri: &str) -> String {
    let (scheme, after_scheme) = match uri.split_once("://") {
        Some((s, rest)) => (s, rest),
        None => return format!("vlinder model add {}", uri),
    };

    match scheme {
        "ollama" => {
            // Strip authority (host:port) to get the model name, then strip :tag
            let name = after_scheme
                .split_once('/')
                .map(|(_, path)| path)
                .unwrap_or(after_scheme);
            let name = name.split(':').next().unwrap_or(name);
            format!("vlinder model add {}", name)
        }
        "openrouter" => {
            // The entire after-scheme part is the model id (e.g. "anthropic/claude-sonnet-4")
            format!("vlinder model add {} --catalog openrouter", after_scheme)
        }
        _ => {
            // Unknown scheme — best effort: last path segment
            let name = after_scheme.rsplit('/').next().unwrap_or(after_scheme);
            format!("vlinder model add {}", name)
        }
    }
}

// ============================================================================
// Registry Trait
// ============================================================================

/// Source of truth for all system state.
///
/// All methods take `&self` — implementations handle internal synchronization.
/// Returns owned values (not references) for network compatibility.
pub trait Registry: Send + Sync {
    /// URI where this registry exposes its API.
    fn id(&self) -> ResourceId;

    // --- Agent operations ---

    /// Register an agent after validating requirements.
    /// Assigns registry identity `<registry_id>/agents/<name>`.
    fn register_agent(&self, agent: Agent) -> Result<(), RegistrationError>;

    /// Get the registry-issued ID for an agent name.
    fn agent_id(&self, name: &str) -> ResourceId;

    /// Get an agent by ID.
    fn get_agent(&self, id: &ResourceId) -> Option<Agent>;

    /// Get all registered agents.
    fn get_agents(&self) -> Vec<Agent>;

    /// Get an agent by name.
    fn get_agent_by_name(&self, name: &str) -> Option<Agent> {
        self.get_agents().into_iter().find(|a| a.name == name)
    }

    /// Select the appropriate runtime for an agent.
    fn select_runtime(&self, agent: &Agent) -> Option<RuntimeType>;

    // --- Model operations ---

    /// Register a model (assigns registry-issued identity).
    fn register_model(&self, model: Model) -> Result<(), RegistrationError>;

    /// Get a model by name.
    fn get_model(&self, name: &str) -> Option<Model>;

    /// Get all registered models.
    fn get_models(&self) -> Vec<Model>;

    /// Get a model by its model_path (the URI that identifies the actual model resource).
    fn get_model_by_path(&self, path: &ResourceId) -> Option<Model>;

    /// Get the registry-issued ID for a model name.
    fn model_id(&self, name: &str) -> ResourceId;

    /// Delete a model by name. Returns true if the model existed.
    fn delete_model(&self, name: &str) -> Result<bool, RegistrationError>;

    // --- Job operations ---

    /// Create a new job with submission tracking (ADR 044).
    fn create_job(&self, submission_id: super::SubmissionId, agent_id: ResourceId, input: String) -> JobId;

    /// Get a job by ID.
    fn get_job(&self, id: &JobId) -> Option<Job>;

    /// Update job status.
    fn update_job_status(&self, id: &JobId, status: JobStatus);

    /// Get all pending jobs.
    fn pending_jobs(&self) -> Vec<Job>;

    // --- Capability registration ---

    fn register_runtime(&self, runtime_type: RuntimeType);
    fn register_object_storage(&self, storage_type: ObjectStorageType);
    fn register_vector_storage(&self, storage_type: VectorStorageType);
    fn register_inference_engine(&self, engine_type: EngineType);
    fn register_embedding_engine(&self, engine_type: EngineType);

    // --- Capability queries ---

    fn has_object_storage(&self, storage_type: ObjectStorageType) -> bool;
    fn has_vector_storage(&self, storage_type: VectorStorageType) -> bool;
    fn has_inference_engine(&self, engine_type: EngineType) -> bool;
    fn has_embedding_engine(&self, engine_type: EngineType) -> bool;
}
