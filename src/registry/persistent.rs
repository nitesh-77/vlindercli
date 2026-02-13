//! PersistentRegistry — write-through registry with SQLite persistence.
//!
//! Composes InMemoryRegistry (fast reads) with SqliteRegistryRepository
//! (durable writes). SQLite is the source of truth; in-memory is a cache.
//!
//! On construction, loads all models and agents from disk (fail-fast).
//! On register_model()/register_agent(), writes to disk first, then updates cache.

use std::path::Path;

use crate::config::Config;
use crate::domain::{
    Agent, EngineType, Job, JobId, JobStatus, Model, ObjectStorageType, RegistrationError,
    Registry, RegistryRepository, ResourceId, RuntimeType, SubmissionId, VectorStorageType,
};
use crate::storage::SqliteRegistryRepository;

use super::InMemoryRegistry;

/// Registry with write-through persistence to SQLite.
///
/// SQLite is the source of truth. InMemoryRegistry is the read cache.
/// All writes go to disk first (fail-fast), then update the cache.
pub struct PersistentRegistry {
    inner: InMemoryRegistry,
    repo: SqliteRegistryRepository,
}

impl PersistentRegistry {
    /// Open a persistent registry backed by SQLite at the given path.
    ///
    /// Registers engine capabilities from config first, then loads all
    /// existing models from disk (validating each against available engines).
    /// Fails fast with a clear error on any failure.
    pub fn open(db_path: &Path, config: &Config) -> Result<Self, RegistrationError> {
        let repo = SqliteRegistryRepository::open(db_path)
            .map_err(|e| RegistrationError::Persistence(
                format!("failed to open registry database '{}': {}", db_path.display(), e)
            ))?;

        let inner = InMemoryRegistry::new();

        // Register engine capabilities BEFORE loading models so validation works
        if config.distributed.workers.inference.ollama > 0 {
            inner.register_inference_engine(EngineType::Ollama);
        }
        if config.distributed.workers.inference.openrouter > 0 {
            inner.register_inference_engine(EngineType::OpenRouter);
        }
        if config.distributed.workers.embedding.ollama > 0 {
            inner.register_embedding_engine(EngineType::Ollama);
        }

        // Load persisted models (each validated against registered engines)
        let models = repo.load_models()
            .map_err(|e| RegistrationError::Persistence(
                format!("failed to load models from '{}': {}", db_path.display(), e)
            ))?;

        for model in models {
            inner.register_model(model)?;
        }

        // Load persisted agents (bypasses validation — capabilities not yet registered)
        let agents = repo.load_agents()
            .map_err(|e| RegistrationError::Persistence(
                format!("failed to load agents from '{}': {}", db_path.display(), e)
            ))?;

        for agent in agents {
            inner.restore_agent(agent)?;
        }

        Ok(Self { inner, repo })
    }

}

// ============================================================================
// Registry trait delegation
// ============================================================================

impl Registry for PersistentRegistry {
    fn id(&self) -> ResourceId {
        self.inner.id()
    }

    // --- Agent operations (write-through for mutations) ---

    fn register_agent(&self, agent: Agent) -> Result<(), RegistrationError> {
        // Validate in-memory first (runtime, storage, model checks), then persist
        self.inner.register_agent(agent.clone())?;

        // Write to disk (agent already validated and cached)
        self.repo.save_agent(&agent)
            .map_err(|e| RegistrationError::Persistence(e.to_string()))?;

        Ok(())
    }

    fn agent_id(&self, name: &str) -> ResourceId {
        self.inner.agent_id(name)
    }

    fn get_agent(&self, id: &ResourceId) -> Option<Agent> {
        self.inner.get_agent(id)
    }

    fn get_agents(&self) -> Vec<Agent> {
        self.inner.get_agents()
    }

    fn select_runtime(&self, agent: &Agent) -> Option<RuntimeType> {
        self.inner.select_runtime(agent)
    }

    // --- Model operations (write-through for mutations) ---

    fn register_model(&self, model: Model) -> Result<(), RegistrationError> {
        // Validate in-memory first (engine availability check), then persist
        self.inner.register_model(model.clone())?;

        // Write to disk (model already validated and cached)
        self.repo.save_model(&model)
            .map_err(|e| RegistrationError::Persistence(e.to_string()))?;

        Ok(())
    }

    fn get_model(&self, name: &str) -> Option<Model> {
        self.inner.get_model(name)
    }

    fn get_models(&self) -> Vec<Model> {
        self.inner.get_models()
    }

    fn get_model_by_path(&self, path: &ResourceId) -> Option<Model> {
        self.inner.get_model_by_path(path)
    }

    fn model_id(&self, name: &str) -> ResourceId {
        self.inner.model_id(name)
    }

    fn delete_model(&self, name: &str) -> Result<bool, RegistrationError> {
        // Check if model exists
        let Some(model) = self.inner.get_model(name) else {
            return Ok(false);
        };

        // Check for dependent agents before deleting
        let dependent: Vec<String> = self.inner.get_agents_requiring_model(&model.model_path)
            .into_iter()
            .map(|a| a.name)
            .collect();

        if !dependent.is_empty() {
            return Err(RegistrationError::ModelInUse(name.to_string(), dependent));
        }

        // Disk first, then cache
        let deleted = self.repo.delete_model(name)
            .map_err(|e| RegistrationError::Persistence(e.to_string()))?;

        if deleted {
            self.inner.remove_model(name);
        }

        Ok(deleted)
    }

    // --- Job operations (delegate directly) ---

    fn create_job(&self, submission_id: SubmissionId, agent_id: ResourceId, input: String) -> JobId {
        self.inner.create_job(submission_id, agent_id, input)
    }

    fn get_job(&self, id: &JobId) -> Option<Job> {
        self.inner.get_job(id)
    }

    fn update_job_status(&self, id: &JobId, status: JobStatus) {
        self.inner.update_job_status(id, status)
    }

    fn pending_jobs(&self) -> Vec<Job> {
        self.inner.pending_jobs()
    }

    // --- Capability registration (delegate directly) ---

    fn register_runtime(&self, runtime_type: RuntimeType) {
        self.inner.register_runtime(runtime_type)
    }

    fn register_object_storage(&self, storage_type: ObjectStorageType) {
        self.inner.register_object_storage(storage_type)
    }

    fn register_vector_storage(&self, storage_type: VectorStorageType) {
        self.inner.register_vector_storage(storage_type)
    }

    fn register_inference_engine(&self, engine_type: EngineType) {
        self.inner.register_inference_engine(engine_type)
    }

    fn register_embedding_engine(&self, engine_type: EngineType) {
        self.inner.register_embedding_engine(engine_type)
    }

    // --- Capability queries (delegate directly) ---

    fn has_object_storage(&self, storage_type: ObjectStorageType) -> bool {
        self.inner.has_object_storage(storage_type)
    }

    fn has_vector_storage(&self, storage_type: VectorStorageType) -> bool {
        self.inner.has_vector_storage(storage_type)
    }

    fn has_inference_engine(&self, engine_type: EngineType) -> bool {
        self.inner.has_inference_engine(engine_type)
    }

    fn has_embedding_engine(&self, engine_type: EngineType) -> bool {
        self.inner.has_embedding_engine(engine_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::domain::{ModelType, Requirements, RuntimeType};

    fn test_model(name: &str) -> Model {
        Model {
            id: Model::placeholder_id(name),
            name: name.to_string(),
            model_type: ModelType::Inference,
            engine: EngineType::Ollama,
            model_path: ResourceId::new(format!("ollama://localhost:11434/{}", name)),
            digest: String::new(),
        }
    }

    #[test]
    fn open_creates_db_if_not_exists() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");

        let registry = PersistentRegistry::open(&db_path, &Config::default()).unwrap();
        assert!(registry.get_models().is_empty());
    }

    #[test]
    fn loads_existing_models_on_open() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");

        // Pre-populate SQLite
        {
            let repo = SqliteRegistryRepository::open(&db_path).unwrap();
            repo.save_model(&test_model("llama3")).unwrap();
        }

        let registry = PersistentRegistry::open(&db_path, &Config::default()).unwrap();
        assert!(registry.get_model("llama3").is_some());
    }

    #[test]
    fn register_model_persists_to_disk() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");

        let registry = PersistentRegistry::open(&db_path, &Config::default()).unwrap();
        registry.register_model(test_model("phi3")).unwrap();

        // Verify in-memory
        assert!(registry.get_model("phi3").is_some());

        // Verify on disk: open a fresh repo and check
        let repo = SqliteRegistryRepository::open(&db_path).unwrap();
        let models = repo.load_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "phi3");
    }

    #[test]
    fn delete_model_removes_from_both() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");

        let registry = PersistentRegistry::open(&db_path, &Config::default()).unwrap();
        registry.register_model(test_model("phi3")).unwrap();
        assert!(registry.get_model("phi3").is_some());

        let deleted = registry.delete_model("phi3").unwrap();
        assert!(deleted);

        // Gone from in-memory
        assert!(registry.get_model("phi3").is_none());

        // Gone from disk
        let repo = SqliteRegistryRepository::open(&db_path).unwrap();
        assert!(!repo.model_exists("phi3").unwrap());
    }

    #[test]
    fn delete_nonexistent_model_returns_false() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");

        let registry = PersistentRegistry::open(&db_path, &Config::default()).unwrap();
        let deleted = registry.delete_model("nope").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn open_fails_fast_on_corrupt_db() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");
        std::fs::write(&db_path, b"not a database").unwrap();

        let result = PersistentRegistry::open(&db_path, &Config::default());
        let err = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error for corrupt db"),
        };
        assert!(err.contains("persistence error"), "got: {}", err);
    }

    #[test]
    fn survives_restart() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");

        // First "session": add a model
        {
            let registry = PersistentRegistry::open(&db_path, &Config::default()).unwrap();
            registry.register_model(test_model("llama3")).unwrap();
        }

        // Second "session": model should be there
        {
            let registry = PersistentRegistry::open(&db_path, &Config::default()).unwrap();
            let model = registry.get_model("llama3");
            assert!(model.is_some(), "model should survive restart");
            assert_eq!(model.unwrap().name, "llama3");
        }
    }

    // --- Agent tests ---

    fn test_agent(name: &str) -> Agent {
        Agent {
            id: Agent::placeholder_id(name),
            name: name.to_string(),
            description: format!("{} agent", name),
            source: None,
            runtime: RuntimeType::Container,
            executable: format!("localhost/{}:latest", name),
            image_digest: None,
            object_storage: None,
            vector_storage: None,
            requirements: Requirements {
                models: HashMap::new(),
                services: vec![],
            },
            prompts: None,
            mounts: vec![],
        }
    }

    /// Config with container runtime enabled so register_agent validation passes.
    fn config_with_runtime() -> Config {
        Config::default()
    }

    /// Open a PersistentRegistry with container runtime pre-registered.
    fn open_with_runtime(db_path: &std::path::Path) -> PersistentRegistry {
        let registry = PersistentRegistry::open(db_path, &config_with_runtime()).unwrap();
        registry.register_runtime(RuntimeType::Container);
        registry
    }

    #[test]
    fn register_agent_persists_to_disk() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");

        let registry = open_with_runtime(&db_path);
        registry.register_agent(test_agent("echo")).unwrap();

        // Verify in-memory
        let agent = registry.get_agent_by_name("echo");
        assert!(agent.is_some());

        // Verify on disk: open a fresh repo and check
        let repo = SqliteRegistryRepository::open(&db_path).unwrap();
        let agents = repo.load_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "echo");
    }

    #[test]
    fn agents_survive_restart() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("registry.db");

        // First "session": register an agent
        {
            let registry = open_with_runtime(&db_path);
            registry.register_agent(test_agent("echo")).unwrap();
        }

        // Second "session": agent should be loaded via restore_agent
        {
            let registry = PersistentRegistry::open(&db_path, &Config::default()).unwrap();
            let agent = registry.get_agent_by_name("echo");
            assert!(agent.is_some(), "agent should survive restart");
            assert_eq!(agent.unwrap().name, "echo");
        }
    }
}
