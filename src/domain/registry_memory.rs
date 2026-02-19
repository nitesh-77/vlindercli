//! InMemoryRegistry — in-process Registry implementation with RwLock.
//!
//! Used directly for tests and as the read cache inside PersistentRegistry.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use super::{
    Agent, Job, JobId, JobStatus, Model, ModelType,
    ObjectStorageType, Provider, RegistrationError, Registry, ResourceId, RuntimeType,
    SecretStore, ServiceType, SubmissionId, VectorStorageType,
    ensure_agent_identity,
};

/// Internal state for InMemoryRegistry.
struct RegistryState {
    jobs: HashMap<JobId, Job>,
    agents: HashMap<ResourceId, Agent>,
    models: HashMap<ResourceId, Model>,
    available_runtimes: HashSet<RuntimeType>,
    available_object_storage: HashSet<ObjectStorageType>,
    available_vector_storage: HashSet<VectorStorageType>,
    available_inference_engines: HashSet<Provider>,
    available_embedding_engines: HashSet<Provider>,
}

/// In-memory registry with internal RwLock for thread-safe access.
pub struct InMemoryRegistry {
    /// URI where this registry exposes its API.
    registry_id: ResourceId,
    state: RwLock<RegistryState>,
    /// Secret store for agent identity provisioning (ADR 084).
    secret_store: Arc<dyn SecretStore>,
}

impl InMemoryRegistry {
    pub fn new(secret_store: Arc<dyn SecretStore>) -> Self {
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
            secret_store,
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

    /// Restore a previously-persisted agent without capability validation.
    ///
    /// Used during startup to load agents from disk. Bypasses runtime,
    /// storage, and model checks — persisted agents already passed
    /// validation at registration time.
    pub fn restore_agent(&self, mut agent: Agent) -> Result<(), RegistrationError> {
        let mut state = self.state.write().unwrap();
        let agent_id = self.agent_id_internal(&agent.name);
        if state.agents.contains_key(&agent_id) {
            return Err(RegistrationError::DuplicateName(agent.name.clone()));
        }
        agent.id = agent_id.clone();
        state.agents.insert(agent_id, agent);
        Ok(())
    }

    /// Remove a model by name. Returns true if the model existed.
    pub fn remove_model(&self, name: &str) -> bool {
        let model_id = ResourceId::new(format!("{}/models/{}", self.registry_id.as_str(), name));
        let mut state = self.state.write().unwrap();
        state.models.remove(&model_id).is_some()
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

        // Validate all declared models are registered and their engines are available (ADR 094)
        for (model_alias, model_name) in &agent.requirements.models {
            let model = state.models.values()
                .find(|m| m.name == *model_name);
            let Some(model) = model else {
                return Err(RegistrationError::ModelNotRegistered(model_alias.clone(), model_name.clone()));
            };

            match model.model_type {
                ModelType::Inference => {
                    if !state.available_inference_engines.contains(&model.provider) {
                        return Err(RegistrationError::InferenceEngineUnavailable(
                            model.provider, model.name.clone(),
                        ));
                    }
                    if !agent.requirements.services.contains_key(&ServiceType::Infer) {
                        return Err(RegistrationError::InferenceServiceNotDeclared(model_alias.clone()));
                    }
                }
                ModelType::Embedding => {
                    if !state.available_embedding_engines.contains(&model.provider) {
                        return Err(RegistrationError::EmbeddingEngineUnavailable(
                            model.provider, model.name.clone(),
                        ));
                    }
                    if !agent.requirements.services.contains_key(&ServiceType::Embed) {
                        return Err(RegistrationError::EmbeddingServiceNotDeclared(model_alias.clone()));
                    }
                }
            }
        }

        // Validate services have corresponding models
        if agent.requirements.services.contains_key(&ServiceType::Infer) {
            let has_inference_model = agent.requirements.models.values().any(|name| {
                state.models.values().any(|m| m.name == *name && m.model_type == ModelType::Inference)
            });
            if !has_inference_model {
                return Err(RegistrationError::InferenceServiceWithoutModel);
            }
        }
        if agent.requirements.services.contains_key(&ServiceType::Embed) {
            let has_embedding_model = agent.requirements.models.values().any(|name| {
                state.models.values().any(|m| m.name == *name && m.model_type == ModelType::Embedding)
            });
            if !has_embedding_model {
                return Err(RegistrationError::EmbeddingServiceWithoutModel);
            }
        }

        // Provision identity (ADR 084): generate Ed25519 key pair, store private key
        let identity = ensure_agent_identity(&agent.name, self.secret_store.as_ref())
            .map_err(|e| RegistrationError::IdentityFailed(e.to_string()))?;
        agent.public_key = Some(identity.public_key.to_vec());

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
                if !state.available_inference_engines.contains(&model.provider) {
                    return Err(RegistrationError::InferenceEngineUnavailable(
                        model.provider, model.name,
                    ));
                }
            }
            ModelType::Embedding => {
                if !state.available_embedding_engines.contains(&model.provider) {
                    return Err(RegistrationError::EmbeddingEngineUnavailable(
                        model.provider, model.name,
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
            .filter(|a| a.requirements.models.values().any(|n| n == &model.name))
            .map(|a| a.name.clone())
            .collect();

        if !dependent.is_empty() {
            return Err(RegistrationError::ModelInUse(name.to_string(), dependent));
        }

        state.models.remove(&model_id);
        Ok(true)
    }

    // --- Job operations ---

    fn create_job(&self, submission_id: SubmissionId, agent_id: ResourceId, input: String) -> JobId {
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

    fn register_inference_engine(&self, engine_type: Provider) {
        let mut state = self.state.write().unwrap();
        state.available_inference_engines.insert(engine_type);
    }

    fn register_embedding_engine(&self, engine_type: Provider) {
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

    fn has_inference_engine(&self, engine_type: Provider) -> bool {
        let state = self.state.read().unwrap();
        state.available_inference_engines.contains(&engine_type)
    }

    fn has_embedding_engine(&self, engine_type: Provider) -> bool {
        let state = self.state.read().unwrap();
        state.available_embedding_engines.contains(&engine_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::secret_store::InMemorySecretStore;

    fn test_secret_store() -> Arc<dyn SecretStore> {
        Arc::new(InMemorySecretStore::new())
    }

    fn test_agent_id() -> ResourceId {
        ResourceId::new("http://127.0.0.1:9000/agents/test-agent")
    }

    #[test]
    fn registry_has_api_endpoint() {
        let registry = InMemoryRegistry::new(test_secret_store());

        // Registry exposes an HTTP API endpoint
        let id = registry.id();
        assert_eq!(id.scheme(), Some("http"));
        assert!(id.as_str().starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn job_id_includes_registry_id() {
        let registry = InMemoryRegistry::new(test_secret_store());
        let agent_id = test_agent_id();

        let job_id = registry.create_job(SubmissionId::new(), agent_id, "test".to_string());

        // JobId format: <registry_id>/jobs/<uuid>
        assert!(job_id.as_str().starts_with(registry.id().as_str()));
        assert!(job_id.as_str().contains("/jobs/"));
    }

    #[test]
    fn job_lifecycle() {
        let registry = InMemoryRegistry::new(test_secret_store());
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
        let registry = InMemoryRegistry::new(test_secret_store());
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
        let registry = InMemoryRegistry::new(test_secret_store());

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
        let registry = InMemoryRegistry::new(test_secret_store());

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
        let registry = InMemoryRegistry::new(test_secret_store());

        // Initially nothing available
        assert!(!registry.has_inference_engine(Provider::Ollama));
        assert!(!registry.has_inference_engine(Provider::InMemory));

        // Register Ollama
        registry.register_inference_engine(Provider::Ollama);
        assert!(registry.has_inference_engine(Provider::Ollama));
        assert!(!registry.has_inference_engine(Provider::InMemory));

        // Register InMemory
        registry.register_inference_engine(Provider::InMemory);
        assert!(registry.has_inference_engine(Provider::Ollama));
        assert!(registry.has_inference_engine(Provider::InMemory));
    }

    // --- Embedding engine tests ---

    #[test]
    fn register_embedding_engine_types() {
        let registry = InMemoryRegistry::new(test_secret_store());

        // Initially nothing available
        assert!(!registry.has_embedding_engine(Provider::Ollama));
        assert!(!registry.has_embedding_engine(Provider::InMemory));

        // Register Ollama
        registry.register_embedding_engine(Provider::Ollama);
        assert!(registry.has_embedding_engine(Provider::Ollama));
        assert!(!registry.has_embedding_engine(Provider::InMemory));

        // Register InMemory
        registry.register_embedding_engine(Provider::InMemory);
        assert!(registry.has_embedding_engine(Provider::Ollama));
        assert!(registry.has_embedding_engine(Provider::InMemory));
    }

    // --- restore_agent tests ---

    fn minimal_agent(name: &str) -> Agent {
        use std::collections::HashMap;
        use super::super::{Requirements, RuntimeType};

        Agent {
            id: Agent::placeholder_id(name),
            name: name.to_string(),
            description: format!("{} agent", name),
            source: None,
            runtime: RuntimeType::Container,
            executable: format!("localhost/{}:latest", name),
            image_digest: None,
            public_key: None,
            object_storage: None,
            vector_storage: None,
            requirements: Requirements {
                models: HashMap::new(),
                services: HashMap::new(),
            },
            prompts: None,
            mounts: vec![],
        }
    }

    #[test]
    fn restore_agent_bypasses_validation() {
        let registry = InMemoryRegistry::new(test_secret_store());
        // No runtimes, storage, or models registered — register_agent would fail
        let agent = minimal_agent("echo");

        // register_agent should fail (no container runtime registered)
        let result = registry.register_agent(agent.clone());
        assert!(result.is_err());

        // restore_agent should succeed despite no capabilities
        registry.restore_agent(agent).unwrap();

        let agents = registry.get_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "echo");
        // ID should be reassigned by the registry
        assert!(agents[0].id.as_str().contains("/agents/echo"));
    }

    #[test]
    fn restore_agent_rejects_duplicate() {
        let registry = InMemoryRegistry::new(test_secret_store());

        registry.restore_agent(minimal_agent("echo")).unwrap();

        let result = registry.restore_agent(minimal_agent("echo"));
        assert!(result.is_err());
    }
}
