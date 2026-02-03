//! Registry - source of truth for all system state.
//!
//! Stores:
//! - Runtimes (available at `/runtimes`)
//! - Models (registered model definitions)
//! - Agents (registered agent definitions)
//! - Jobs (submitted, running, completed)

use std::collections::{HashMap, HashSet};

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

/// The registry - source of truth for all state.
pub struct Registry {
    /// URI where this registry exposes its API.
    pub id: ResourceId,
    jobs: HashMap<JobId, Job>,
    agents: HashMap<ResourceId, Agent>,
    models: HashMap<ResourceId, Model>,
    available_runtimes: HashSet<RuntimeType>,
    available_object_storage: HashSet<ObjectStorageType>,
    available_vector_storage: HashSet<VectorStorageType>,
    available_inference_engines: HashSet<EngineType>,
    available_embedding_engines: HashSet<EngineType>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            id: ResourceId::new("http://127.0.0.1:9000"),
            jobs: HashMap::new(),
            agents: HashMap::new(),
            models: HashMap::new(),
            available_runtimes: HashSet::new(),
            available_object_storage: HashSet::new(),
            available_vector_storage: HashSet::new(),
            available_inference_engines: HashSet::new(),
            available_embedding_engines: HashSet::new(),
        }
    }

    // --- Runtime operations ---

    /// Register a runtime type as available.
    pub fn register_runtime(&mut self, runtime_type: RuntimeType) {
        self.available_runtimes.insert(runtime_type);
    }

    /// Select the appropriate runtime for an agent based on its id.
    ///
    /// Returns None if no suitable runtime is available.
    pub fn select_runtime(&self, agent: &Agent) -> Option<RuntimeType> {
        // file:// scheme + .wasm extension → Wasm runtime
        if agent.id.scheme() == Some("file") {
            if let Some(path) = agent.id.path() {
                if path.ends_with(".wasm") && self.available_runtimes.contains(&RuntimeType::Wasm) {
                    return Some(RuntimeType::Wasm);
                }
            }
        }
        None
    }

    // --- Object storage operations ---

    /// Register an object storage type as available.
    pub fn register_object_storage(&mut self, storage_type: ObjectStorageType) {
        self.available_object_storage.insert(storage_type);
    }

    /// Check if an object storage type is available.
    pub fn has_object_storage(&self, storage_type: ObjectStorageType) -> bool {
        self.available_object_storage.contains(&storage_type)
    }

    // --- Vector storage operations ---

    /// Register a vector storage type as available.
    pub fn register_vector_storage(&mut self, storage_type: VectorStorageType) {
        self.available_vector_storage.insert(storage_type);
    }

    /// Check if a vector storage type is available.
    pub fn has_vector_storage(&self, storage_type: VectorStorageType) -> bool {
        self.available_vector_storage.contains(&storage_type)
    }

    // --- Inference engine operations ---

    /// Register an inference engine type as available.
    pub fn register_inference_engine(&mut self, engine_type: EngineType) {
        self.available_inference_engines.insert(engine_type);
    }

    /// Check if an inference engine type is available.
    pub fn has_inference_engine(&self, engine_type: EngineType) -> bool {
        self.available_inference_engines.contains(&engine_type)
    }

    // --- Embedding engine operations ---

    /// Register an embedding engine type as available.
    pub fn register_embedding_engine(&mut self, engine_type: EngineType) {
        self.available_embedding_engines.insert(engine_type);
    }

    /// Check if an embedding engine type is available.
    pub fn has_embedding_engine(&self, engine_type: EngineType) -> bool {
        self.available_embedding_engines.contains(&engine_type)
    }

    // --- Job operations ---

    pub fn create_job(&mut self, agent_id: ResourceId, input: String) -> JobId {
        let id = JobId::new(&self.id);
        let job = Job {
            id: id.clone(),
            agent_id,
            input,
            status: JobStatus::Pending,
        };
        self.jobs.insert(id.clone(), job);
        id
    }

    pub fn get_job(&self, id: &JobId) -> Option<&Job> {
        self.jobs.get(id)
    }

    pub fn update_job_status(&mut self, id: &JobId, status: JobStatus) {
        if let Some(job) = self.jobs.get_mut(id) {
            job.status = status;
        }
    }

    pub fn pending_jobs(&self) -> Vec<&Job> {
        self.jobs
            .values()
            .filter(|j| j.status == JobStatus::Pending)
            .collect()
    }

    // --- Model operations ---

    /// Register a model as available.
    ///
    /// Assigns the model's identity: `<registry_id>/models/<name>`.
    pub fn register_model(&mut self, mut model: Model) {
        let model_id = self.model_id(&model.name);
        model.id = model_id.clone();
        self.models.insert(model_id, model);
    }

    /// Get a registered model by name.
    pub fn get_model(&self, name: &str) -> Option<&Model> {
        let model_id = self.model_id(name);
        self.models.get(&model_id)
    }

    /// Get the registry-issued ID for a model.
    ///
    /// Format: `<registry_id>/models/<name>`
    pub fn model_id(&self, name: &str) -> ResourceId {
        ResourceId::new(format!("{}/models/{}", self.id.as_str(), name))
    }

    // --- Agent operations ---

    /// Register an agent after validating all its requirements can be met.
    ///
    /// Validates:
    /// - A runtime is available for the agent's scheme/extension
    /// - Declared object storage scheme is supported
    /// - Declared vector storage scheme is supported
    /// - All declared models are registered
    pub fn register_agent(&mut self, agent: Agent) -> Result<(), RegistrationError> {
        // Validate runtime
        if self.select_runtime(&agent).is_none() {
            return Err(RegistrationError::NoRuntime(agent.id.clone()));
        }

        // Validate object storage if declared
        if let Some(ref uri) = agent.object_storage {
            let scheme = uri.scheme().unwrap_or("unknown");
            let storage_type = ObjectStorageType::from_scheme(uri.scheme())
                .ok_or_else(|| RegistrationError::UnknownObjectStorageScheme(scheme.to_string()))?;
            if !self.has_object_storage(storage_type) {
                return Err(RegistrationError::ObjectStorageUnavailable(storage_type));
            }
        }

        // Validate vector storage if declared
        if let Some(ref uri) = agent.vector_storage {
            let scheme = uri.scheme().unwrap_or("unknown");
            let storage_type = VectorStorageType::from_scheme(uri.scheme())
                .ok_or_else(|| RegistrationError::UnknownVectorStorageScheme(scheme.to_string()))?;
            if !self.has_vector_storage(storage_type) {
                return Err(RegistrationError::VectorStorageUnavailable(storage_type));
            }
        }

        // Validate all declared models are registered (by name)
        for (model_name, _manifest_uri) in &agent.requirements.models {
            if self.get_model(model_name).is_none() {
                return Err(RegistrationError::ModelNotRegistered(model_name.clone()));
            }
        }

        // All validated - store agent
        self.agents.insert(agent.id.clone(), agent);
        Ok(())
    }

    pub fn get_agent(&self, id: &ResourceId) -> Option<&Agent> {
        self.agents.get(id)
    }

    /// Get all registered agents.
    pub fn get_agents(&self) -> impl Iterator<Item = &Agent> {
        self.agents.values()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
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
        let registry = Registry::new();

        // Registry exposes an HTTP API endpoint
        assert_eq!(registry.id.scheme(), Some("http"));
        assert!(registry.id.as_str().starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn job_id_includes_registry_id() {
        let mut registry = Registry::new();
        let agent_id = test_agent_id();

        let job_id = registry.create_job(agent_id, "test".to_string());

        // JobId format: <registry_id>/jobs/<uuid>
        assert!(job_id.as_str().starts_with(registry.id.as_str()));
        assert!(job_id.as_str().contains("/jobs/"));
    }

    #[test]
    fn job_lifecycle() {
        let mut registry = Registry::new();
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
        let mut registry = Registry::new();
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
        let mut registry = Registry::new();

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
        let mut registry = Registry::new();

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
        let mut registry = Registry::new();

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
        let mut registry = Registry::new();

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
