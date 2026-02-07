//! Registry - source of truth for all system state.
//!
//! Stores:
//! - Runtimes (available at `/runtimes`)
//! - Models (registered model definitions)
//! - Agents (registered agent definitions)
//! - Jobs (submitted, running, completed)
//!
//! The `Registry` trait abstracts state storage. Implementations:
//! - `InMemoryRegistry` — in-process with RwLock (current)
//! - Future: HTTP/gRPC client for distributed deployment

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use crate::domain::{Agent, EngineType, Model, ModelType, ObjectStorageType, ResourceId, RuntimeType, VectorStorageType};

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
    pub submission_id: crate::queue::SubmissionId,  // ADR 044: tracks message flow
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
    ModelNotRegistered(String),
    /// Agent requires an inference engine that is not available.
    InferenceEngineUnavailable(EngineType, String),
    /// Agent requires an embedding engine that is not available.
    EmbeddingEngineUnavailable(EngineType, String),
    /// Model cannot be removed because deployed agents depend on it.
    ModelInUse(String, Vec<String>),
    /// Persistence operation failed (disk I/O, database error, etc.).
    Persistence(String),
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistrationError::DuplicateName(name) => write!(f, "agent already registered: {}", name),
            RegistrationError::NoRuntime(id) => write!(f, "no runtime available for agent executable: {}", id),
            RegistrationError::UnknownObjectStorageScheme(s) => write!(f, "unknown object storage scheme: {}", s),
            RegistrationError::ObjectStorageUnavailable(t) => write!(f, "object storage not available: {:?}", t),
            RegistrationError::UnknownVectorStorageScheme(s) => write!(f, "unknown vector storage scheme: {}", s),
            RegistrationError::VectorStorageUnavailable(t) => write!(f, "vector storage not available: {:?}", t),
            RegistrationError::ModelNotRegistered(name) => write!(f, "model not registered: {}", name),
            RegistrationError::InferenceEngineUnavailable(engine, model) => write!(f, "inference engine {:?} not available (required by model '{}')", engine, model),
            RegistrationError::EmbeddingEngineUnavailable(engine, model) => write!(f, "embedding engine {:?} not available (required by model '{}')", engine, model),
            RegistrationError::ModelInUse(name, agents) => write!(f, "model '{}' is in use by agents: {}", name, agents.join(", ")),
            RegistrationError::Persistence(msg) => write!(f, "persistence error: {}", msg),
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
    fn create_job(&self, submission_id: crate::queue::SubmissionId, agent_id: ResourceId, input: String) -> JobId;

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

// ============================================================================
// In-Memory Implementation
// ============================================================================

/// Internal state for InMemoryRegistry.
struct RegistryState {
    jobs: HashMap<JobId, Job>,
    agents: HashMap<ResourceId, Agent>,
    models: HashMap<ResourceId, Model>,
    available_runtimes: HashSet<RuntimeType>,
    available_object_storage: HashSet<ObjectStorageType>,
    available_vector_storage: HashSet<VectorStorageType>,
    available_inference_engines: HashSet<EngineType>,
    available_embedding_engines: HashSet<EngineType>,
}

/// In-memory registry with internal RwLock for thread-safe access.
pub struct InMemoryRegistry {
    /// URI where this registry exposes its API.
    registry_id: ResourceId,
    state: RwLock<RegistryState>,
}

impl InMemoryRegistry {
    pub fn new() -> Self {
        Self {
            registry_id: ResourceId::new("http://127.0.0.1:9000"),
            state: RwLock::new(RegistryState {
                jobs: HashMap::new(),
                agents: HashMap::new(),
                models: HashMap::new(),
                available_runtimes: HashSet::new(),
                available_object_storage: HashSet::new(),
                available_vector_storage: HashSet::new(),
                available_inference_engines: HashSet::new(),
                available_embedding_engines: HashSet::new(),
            }),
        }
    }

    /// Internal: check if runtime is available for agent (needs read lock held).
    fn select_runtime_internal(&self, agent: &Agent, state: &RegistryState) -> Option<RuntimeType> {
        if state.available_runtimes.contains(&agent.runtime) {
            Some(agent.runtime)
        } else {
            None
        }
    }

    /// Internal: compute agent ID without needing &self for borrow checker.
    fn agent_id_internal(&self, name: &str) -> ResourceId {
        ResourceId::new(format!("{}/agents/{}", self.registry_id.as_str(), name))
    }

    /// Remove a model by name. Returns true if the model existed.
    pub fn remove_model(&self, name: &str) -> bool {
        let model_id = ResourceId::new(format!("{}/models/{}", self.registry_id.as_str(), name));
        let mut state = self.state.write().unwrap();
        state.models.remove(&model_id).is_some()
    }
}

impl Default for InMemoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry for InMemoryRegistry {
    fn id(&self) -> ResourceId {
        self.registry_id.clone()
    }

    // --- Agent operations ---

    fn register_agent(&self, mut agent: Agent) -> Result<(), RegistrationError> {
        let mut state = self.state.write().unwrap();

        // Check name uniqueness
        let agent_id = self.agent_id_internal(&agent.name);
        if state.agents.contains_key(&agent_id) {
            return Err(RegistrationError::DuplicateName(agent.name.clone()));
        }

        // Validate runtime is available
        if self.select_runtime_internal(&agent, &state).is_none() {
            return Err(RegistrationError::NoRuntime(
                ResourceId::new(format!("runtime:{}", agent.runtime.as_str()))
            ));
        }

        // Validate object storage if declared
        if let Some(ref uri) = agent.object_storage {
            let scheme = uri.scheme().unwrap_or("unknown");
            let storage_type = ObjectStorageType::from_scheme(uri.scheme())
                .ok_or_else(|| RegistrationError::UnknownObjectStorageScheme(scheme.to_string()))?;
            if !state.available_object_storage.contains(&storage_type) {
                return Err(RegistrationError::ObjectStorageUnavailable(storage_type));
            }
        }

        // Validate vector storage if declared
        if let Some(ref uri) = agent.vector_storage {
            let scheme = uri.scheme().unwrap_or("unknown");
            let storage_type = VectorStorageType::from_scheme(uri.scheme())
                .ok_or_else(|| RegistrationError::UnknownVectorStorageScheme(scheme.to_string()))?;
            if !state.available_vector_storage.contains(&storage_type) {
                return Err(RegistrationError::VectorStorageUnavailable(storage_type));
            }
        }

        // Validate all declared models are registered and their engines are available
        for (model_alias, manifest_uri) in &agent.requirements.models {
            let model = state.models.values()
                .find(|m| &m.model_path == manifest_uri);
            let Some(model) = model else {
                return Err(RegistrationError::ModelNotRegistered(model_alias.clone()));
            };

            match model.model_type {
                ModelType::Inference => {
                    if !state.available_inference_engines.contains(&model.engine) {
                        return Err(RegistrationError::InferenceEngineUnavailable(
                            model.engine, model.name.clone(),
                        ));
                    }
                }
                ModelType::Embedding => {
                    if !state.available_embedding_engines.contains(&model.engine) {
                        return Err(RegistrationError::EmbeddingEngineUnavailable(
                            model.engine, model.name.clone(),
                        ));
                    }
                }
            }
        }

        // Assign registry identity and store
        agent.id = agent_id.clone();
        state.agents.insert(agent_id, agent);
        Ok(())
    }

    fn get_agent(&self, id: &ResourceId) -> Option<Agent> {
        let state = self.state.read().unwrap();
        state.agents.get(id).cloned()
    }

    fn get_agents(&self) -> Vec<Agent> {
        let state = self.state.read().unwrap();
        state.agents.values().cloned().collect()
    }

    fn select_runtime(&self, agent: &Agent) -> Option<RuntimeType> {
        let state = self.state.read().unwrap();
        self.select_runtime_internal(agent, &state)
    }

    // --- Model operations ---

    fn register_model(&self, mut model: Model) -> Result<(), RegistrationError> {
        let model_id = self.model_id(&model.name);
        model.id = model_id.clone();
        let mut state = self.state.write().unwrap();

        // Validate that the required engine is available
        match model.model_type {
            ModelType::Inference => {
                if !state.available_inference_engines.contains(&model.engine) {
                    return Err(RegistrationError::InferenceEngineUnavailable(
                        model.engine, model.name,
                    ));
                }
            }
            ModelType::Embedding => {
                if !state.available_embedding_engines.contains(&model.engine) {
                    return Err(RegistrationError::EmbeddingEngineUnavailable(
                        model.engine, model.name,
                    ));
                }
            }
        }

        state.models.insert(model_id, model);
        Ok(())
    }

    fn get_model(&self, name: &str) -> Option<Model> {
        let model_id = self.model_id(name);
        let state = self.state.read().unwrap();
        state.models.get(&model_id).cloned()
    }

    fn get_models(&self) -> Vec<Model> {
        let state = self.state.read().unwrap();
        state.models.values().cloned().collect()
    }

    fn get_model_by_path(&self, path: &ResourceId) -> Option<Model> {
        let state = self.state.read().unwrap();
        state.models.values()
            .find(|m| &m.model_path == path)
            .cloned()
    }

    fn agent_id(&self, name: &str) -> ResourceId {
        self.agent_id_internal(name)
    }

    fn model_id(&self, name: &str) -> ResourceId {
        ResourceId::new(format!("{}/models/{}", self.registry_id.as_str(), name))
    }

    fn delete_model(&self, name: &str) -> Result<bool, RegistrationError> {
        let model_id = self.model_id(name);
        let mut state = self.state.write().unwrap();

        let Some(model) = state.models.get(&model_id) else {
            return Ok(false);
        };

        let dependent: Vec<String> = state.agents.values()
            .filter(|a| a.requirements.models.values().any(|uri| uri == &model.model_path))
            .map(|a| a.name.clone())
            .collect();

        if !dependent.is_empty() {
            return Err(RegistrationError::ModelInUse(name.to_string(), dependent));
        }

        state.models.remove(&model_id);
        Ok(true)
    }

    // --- Job operations ---

    fn create_job(&self, submission_id: crate::queue::SubmissionId, agent_id: ResourceId, input: String) -> JobId {
        let id = JobId::new(&self.registry_id);
        let job = Job {
            id: id.clone(),
            submission_id,
            agent_id,
            input,
            status: JobStatus::Pending,
        };
        let mut state = self.state.write().unwrap();
        state.jobs.insert(id.clone(), job);
        id
    }

    fn get_job(&self, id: &JobId) -> Option<Job> {
        let state = self.state.read().unwrap();
        state.jobs.get(id).cloned()
    }

    fn update_job_status(&self, id: &JobId, status: JobStatus) {
        let mut state = self.state.write().unwrap();
        if let Some(job) = state.jobs.get_mut(id) {
            job.status = status;
        }
    }

    fn pending_jobs(&self) -> Vec<Job> {
        let state = self.state.read().unwrap();
        state.jobs
            .values()
            .filter(|j| j.status == JobStatus::Pending)
            .cloned()
            .collect()
    }

    // --- Capability registration ---

    fn register_runtime(&self, runtime_type: RuntimeType) {
        let mut state = self.state.write().unwrap();
        state.available_runtimes.insert(runtime_type);
    }

    fn register_object_storage(&self, storage_type: ObjectStorageType) {
        let mut state = self.state.write().unwrap();
        state.available_object_storage.insert(storage_type);
    }

    fn register_vector_storage(&self, storage_type: VectorStorageType) {
        let mut state = self.state.write().unwrap();
        state.available_vector_storage.insert(storage_type);
    }

    fn register_inference_engine(&self, engine_type: EngineType) {
        let mut state = self.state.write().unwrap();
        state.available_inference_engines.insert(engine_type);
    }

    fn register_embedding_engine(&self, engine_type: EngineType) {
        let mut state = self.state.write().unwrap();
        state.available_embedding_engines.insert(engine_type);
    }

    // --- Capability queries ---

    fn has_object_storage(&self, storage_type: ObjectStorageType) -> bool {
        let state = self.state.read().unwrap();
        state.available_object_storage.contains(&storage_type)
    }

    fn has_vector_storage(&self, storage_type: VectorStorageType) -> bool {
        let state = self.state.read().unwrap();
        state.available_vector_storage.contains(&storage_type)
    }

    fn has_inference_engine(&self, engine_type: EngineType) -> bool {
        let state = self.state.read().unwrap();
        state.available_inference_engines.contains(&engine_type)
    }

    fn has_embedding_engine(&self, engine_type: EngineType) -> bool {
        let state = self.state.read().unwrap();
        state.available_embedding_engines.contains(&engine_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::SubmissionId;

    fn test_agent_id() -> ResourceId {
        ResourceId::new("http://127.0.0.1:9000/agents/test-agent")
    }

    #[test]
    fn registry_has_api_endpoint() {
        let registry = InMemoryRegistry::new();

        // Registry exposes an HTTP API endpoint
        let id = registry.id();
        assert_eq!(id.scheme(), Some("http"));
        assert!(id.as_str().starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn job_id_includes_registry_id() {
        let registry = InMemoryRegistry::new();
        let agent_id = test_agent_id();

        let job_id = registry.create_job(SubmissionId::new(), agent_id, "test".to_string());

        // JobId format: <registry_id>/jobs/<uuid>
        assert!(job_id.as_str().starts_with(registry.id().as_str()));
        assert!(job_id.as_str().contains("/jobs/"));
    }

    #[test]
    fn job_lifecycle() {
        let registry = InMemoryRegistry::new();
        let agent_id = test_agent_id();

        // Create job
        let job_id = registry.create_job(SubmissionId::new(), agent_id.clone(), "hello".to_string());

        // Initial state is Pending
        let job = registry.get_job(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.agent_id, agent_id);
        assert_eq!(job.input, "hello");

        // Update to Running
        registry.update_job_status(&job_id, JobStatus::Running);
        assert_eq!(registry.get_job(&job_id).unwrap().status, JobStatus::Running);

        // Update to Completed
        registry.update_job_status(&job_id, JobStatus::Completed("result".to_string()));
        assert_eq!(
            registry.get_job(&job_id).unwrap().status,
            JobStatus::Completed("result".to_string())
        );
    }

    #[test]
    fn pending_jobs_filters_by_status() {
        let registry = InMemoryRegistry::new();
        let agent_id = test_agent_id();

        let job1 = registry.create_job(SubmissionId::new(), agent_id.clone(), "a".to_string());
        let job2 = registry.create_job(SubmissionId::new(), agent_id.clone(), "b".to_string());
        let _job3 = registry.create_job(SubmissionId::new(), agent_id.clone(), "c".to_string());

        // All three are pending
        assert_eq!(registry.pending_jobs().len(), 3);

        // Mark one as running, one as completed
        registry.update_job_status(&job1, JobStatus::Running);
        registry.update_job_status(&job2, JobStatus::Completed("done".to_string()));

        // Only one pending now
        assert_eq!(registry.pending_jobs().len(), 1);
    }

    // --- Object storage tests ---

    #[test]
    fn register_object_storage_types() {
        let registry = InMemoryRegistry::new();

        // Initially nothing available
        assert!(!registry.has_object_storage(ObjectStorageType::Sqlite));
        assert!(!registry.has_object_storage(ObjectStorageType::InMemory));

        // Register Sqlite
        registry.register_object_storage(ObjectStorageType::Sqlite);
        assert!(registry.has_object_storage(ObjectStorageType::Sqlite));
        assert!(!registry.has_object_storage(ObjectStorageType::InMemory));

        // Register InMemory
        registry.register_object_storage(ObjectStorageType::InMemory);
        assert!(registry.has_object_storage(ObjectStorageType::Sqlite));
        assert!(registry.has_object_storage(ObjectStorageType::InMemory));
    }

    // --- Vector storage tests ---

    #[test]
    fn register_vector_storage_types() {
        let registry = InMemoryRegistry::new();

        // Initially nothing available
        assert!(!registry.has_vector_storage(VectorStorageType::SqliteVec));
        assert!(!registry.has_vector_storage(VectorStorageType::InMemory));

        // Register SqliteVec
        registry.register_vector_storage(VectorStorageType::SqliteVec);
        assert!(registry.has_vector_storage(VectorStorageType::SqliteVec));
        assert!(!registry.has_vector_storage(VectorStorageType::InMemory));

        // Register InMemory
        registry.register_vector_storage(VectorStorageType::InMemory);
        assert!(registry.has_vector_storage(VectorStorageType::SqliteVec));
        assert!(registry.has_vector_storage(VectorStorageType::InMemory));
    }

    // --- Inference engine tests ---

    #[test]
    fn register_inference_engine_types() {
        let registry = InMemoryRegistry::new();

        // Initially nothing available
        assert!(!registry.has_inference_engine(EngineType::Llama));
        assert!(!registry.has_inference_engine(EngineType::InMemory));

        // Register Llama
        registry.register_inference_engine(EngineType::Llama);
        assert!(registry.has_inference_engine(EngineType::Llama));
        assert!(!registry.has_inference_engine(EngineType::InMemory));

        // Register InMemory
        registry.register_inference_engine(EngineType::InMemory);
        assert!(registry.has_inference_engine(EngineType::Llama));
        assert!(registry.has_inference_engine(EngineType::InMemory));
    }

    // --- Embedding engine tests ---

    #[test]
    fn register_embedding_engine_types() {
        let registry = InMemoryRegistry::new();

        // Initially nothing available
        assert!(!registry.has_embedding_engine(EngineType::Llama));
        assert!(!registry.has_embedding_engine(EngineType::InMemory));

        // Register Llama
        registry.register_embedding_engine(EngineType::Llama);
        assert!(registry.has_embedding_engine(EngineType::Llama));
        assert!(!registry.has_embedding_engine(EngineType::InMemory));

        // Register InMemory
        registry.register_embedding_engine(EngineType::InMemory);
        assert!(registry.has_embedding_engine(EngineType::Llama));
        assert!(registry.has_embedding_engine(EngineType::InMemory));
    }
}
