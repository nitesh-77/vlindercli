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

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A submitted job and its current state.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: JobId,
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
    /// No runtime available for this agent's scheme/extension.
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
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistrationError::NoRuntime(id) => write!(f, "no runtime available for agent: {}", id),
            RegistrationError::UnknownObjectStorageScheme(s) => write!(f, "unknown object storage scheme: {}", s),
            RegistrationError::ObjectStorageUnavailable(t) => write!(f, "object storage not available: {:?}", t),
            RegistrationError::UnknownVectorStorageScheme(s) => write!(f, "unknown vector storage scheme: {}", s),
            RegistrationError::VectorStorageUnavailable(t) => write!(f, "vector storage not available: {:?}", t),
            RegistrationError::ModelNotRegistered(name) => write!(f, "model not registered: {}", name),
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
    fn register_agent(&self, agent: Agent) -> Result<(), RegistrationError>;

    /// Get an agent by ID.
    fn get_agent(&self, id: &ResourceId) -> Option<Agent>;

    /// Get all registered agents.
    fn get_agents(&self) -> Vec<Agent>;

    /// Select the appropriate runtime for an agent.
    fn select_runtime(&self, agent: &Agent) -> Option<RuntimeType>;

    // --- Model operations ---

    /// Register a model (assigns registry-issued identity).
    fn register_model(&self, model: Model);

    /// Get a model by name.
    fn get_model(&self, name: &str) -> Option<Model>;

    /// Get the registry-issued ID for a model name.
    fn model_id(&self, name: &str) -> ResourceId;

    // --- Job operations ---

    /// Create a new job.
    fn create_job(&self, agent_id: ResourceId, input: String) -> JobId;

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
        if agent.id.scheme() == Some("file") {
            if let Some(path) = agent.id.path() {
                if path.ends_with(".wasm") && state.available_runtimes.contains(&RuntimeType::Wasm) {
                    return Some(RuntimeType::Wasm);
                }
            }
        }
        None
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

    fn register_agent(&self, agent: Agent) -> Result<(), RegistrationError> {
        let mut state = self.state.write().unwrap();

        // Validate runtime
        if self.select_runtime_internal(&agent, &state).is_none() {
            return Err(RegistrationError::NoRuntime(agent.id.clone()));
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

        // Validate all declared models are registered (by name)
        let model_id_prefix = format!("{}/models/", self.registry_id.as_str());
        for (model_name, _manifest_uri) in &agent.requirements.models {
            let model_id = ResourceId::new(format!("{}{}", model_id_prefix, model_name));
            if !state.models.contains_key(&model_id) {
                return Err(RegistrationError::ModelNotRegistered(model_name.clone()));
            }
        }

        // All validated - store agent
        state.agents.insert(agent.id.clone(), agent);
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

    fn register_model(&self, mut model: Model) {
        let model_id = self.model_id(&model.name);
        model.id = model_id.clone();
        let mut state = self.state.write().unwrap();
        state.models.insert(model_id, model);
    }

    fn get_model(&self, name: &str) -> Option<Model> {
        let model_id = self.model_id(name);
        let state = self.state.read().unwrap();
        state.models.get(&model_id).cloned()
    }

    fn model_id(&self, name: &str) -> ResourceId {
        ResourceId::new(format!("{}/models/{}", self.registry_id.as_str(), name))
    }

    // --- Job operations ---

    fn create_job(&self, agent_id: ResourceId, input: String) -> JobId {
        let id = JobId::new(&self.registry_id);
        let job = Job {
            id: id.clone(),
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

    fn test_agent_id() -> ResourceId {
        ResourceId::new("file:///test/agent.wasm")
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

        let job_id = registry.create_job(agent_id, "test".to_string());

        // JobId format: <registry_id>/jobs/<uuid>
        assert!(job_id.as_str().starts_with(registry.id().as_str()));
        assert!(job_id.as_str().contains("/jobs/"));
    }

    #[test]
    fn job_lifecycle() {
        let registry = InMemoryRegistry::new();
        let agent_id = test_agent_id();

        // Create job
        let job_id = registry.create_job(agent_id.clone(), "hello".to_string());

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

        let job1 = registry.create_job(agent_id.clone(), "a".to_string());
        let job2 = registry.create_job(agent_id.clone(), "b".to_string());
        let _job3 = registry.create_job(agent_id.clone(), "c".to_string());

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
