//! `SQLite` implementation of `RegistryRepository`.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use vlinder_core::domain::{
    Agent, Model, ModelType, Provider, RegistryRepository, RepositoryError, StoredAgent,
    StoredModel,
};

/// SQLite-backed registry repository.
pub struct SqliteRegistryRepository {
    conn: Mutex<Connection>,
}

impl SqliteRegistryRepository {
    /// Open or create a registry database at the given path.
    pub fn open(path: &Path) -> Result<Self, RepositoryError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RepositoryError::Database(e.to_string()))?;
        }

        let conn = Connection::open(path).map_err(|e| RepositoryError::Database(e.to_string()))?;

        let repo = Self {
            conn: Mutex::new(conn),
        };
        repo.init_schema()?;
        Ok(repo)
    }

    /// Open an in-memory database (for testing).
    pub fn in_memory() -> Result<Self, RepositoryError> {
        let conn =
            Connection::open_in_memory().map_err(|e| RepositoryError::Database(e.to_string()))?;

        let repo = Self {
            conn: Mutex::new(conn),
        };
        repo.init_schema()?;
        Ok(repo)
    }

    fn init_schema(&self) -> Result<(), RepositoryError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS models (
                name TEXT PRIMARY KEY,
                model_type TEXT NOT NULL,
                provider TEXT NOT NULL,
                model_path TEXT NOT NULL,
                digest TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS agents (
                name TEXT PRIMARY KEY,
                description TEXT NOT NULL,
                source TEXT,
                runtime TEXT NOT NULL,
                executable TEXT NOT NULL,
                image_digest TEXT,
                object_storage TEXT,
                vector_storage TEXT,
                requirements_json TEXT NOT NULL,
                prompts_json TEXT,
                public_key BLOB
            )",
            [],
        )
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(())
    }
}

impl RegistryRepository for SqliteRegistryRepository {
    fn save_model(&self, model: &Model) -> Result<(), RepositoryError> {
        let stored = StoredModel::from_model(model);
        let conn = self.conn.lock().unwrap();

        let model_type: String = serde_json::to_value(&stored.model_type)
            .map_err(|e| RepositoryError::Serialization(e.to_string()))?
            .as_str()
            .unwrap()
            .to_string();
        let provider: String = serde_json::to_value(stored.provider)
            .map_err(|e| RepositoryError::Serialization(e.to_string()))?
            .as_str()
            .unwrap()
            .to_string();

        conn.execute(
            "INSERT OR REPLACE INTO models (name, model_type, provider, model_path, digest)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            [
                &stored.name,
                &model_type,
                &provider,
                &stored.model_path,
                &stored.digest,
            ],
        )
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(())
    }

    fn load_models(&self) -> Result<Vec<Model>, RepositoryError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT name, model_type, provider, model_path, digest FROM models")
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let model_type_str: String = row.get(1)?;
                let provider_str: String = row.get(2)?;
                let model_path: String = row.get(3)?;
                let digest: String = row.get(4)?;

                let model_type: ModelType = serde_json::from_value(serde_json::Value::String(
                    model_type_str,
                ))
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

                let provider: Provider = serde_json::from_value(serde_json::Value::String(
                    provider_str,
                ))
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

                Ok(StoredModel {
                    name,
                    model_type,
                    provider,
                    model_path,
                    digest,
                })
            })
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let mut models = Vec::new();
        for row in rows {
            let stored = row.map_err(|e| RepositoryError::Database(e.to_string()))?;
            models.push(stored.to_model());
        }
        Ok(models)
    }

    fn delete_model(&self, name: &str) -> Result<bool, RepositoryError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn
            .execute("DELETE FROM models WHERE name = ?1", [name])
            .map_err(|e| RepositoryError::Database(e.to_string()))?;
        Ok(affected > 0)
    }

    fn model_exists(&self, name: &str) -> Result<bool, RepositoryError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM models WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .map_err(|e| RepositoryError::Database(e.to_string()))?;
        Ok(count > 0)
    }

    fn save_agent(&self, agent: &Agent) -> Result<(), RepositoryError> {
        let stored = StoredAgent::from_agent(agent)?;
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO agents (name, description, source, runtime, executable,
             image_digest, object_storage, vector_storage, requirements_json, prompts_json,
             public_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                stored.name,
                stored.description,
                stored.source,
                stored.runtime,
                stored.executable,
                stored.image_digest,
                stored.object_storage,
                stored.vector_storage,
                stored.requirements_json,
                stored.prompts_json,
                stored.public_key,
            ],
        )
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(())
    }

    fn load_agents(&self) -> Result<Vec<Agent>, RepositoryError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT name, description, source, runtime, executable,
                 image_digest, object_storage, vector_storage,
                 requirements_json, prompts_json, public_key
                 FROM agents",
            )
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(StoredAgent {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    source: row.get(2)?,
                    runtime: row.get(3)?,
                    executable: row.get(4)?,
                    image_digest: row.get(5)?,
                    object_storage: row.get(6)?,
                    vector_storage: row.get(7)?,
                    requirements_json: row.get(8)?,
                    prompts_json: row.get(9)?,
                    public_key: row.get(10)?,
                })
            })
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let mut agents = Vec::new();
        for row in rows {
            let stored = row.map_err(|e| RepositoryError::Database(e.to_string()))?;
            agents.push(stored.to_agent()?);
        }
        Ok(agents)
    }

    fn delete_agent(&self, name: &str) -> Result<bool, RepositoryError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn
            .execute("DELETE FROM agents WHERE name = ?1", [name])
            .map_err(|e| RepositoryError::Database(e.to_string()))?;
        Ok(affected > 0)
    }

    fn agent_exists(&self, name: &str) -> Result<bool, RepositoryError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agents WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .map_err(|e| RepositoryError::Database(e.to_string()))?;
        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use vlinder_core::domain::{
        Agent, ImageDigest, ModelType, Prompts, Provider, Requirements, ResourceId, RuntimeType,
    };

    fn test_model(name: &str) -> Model {
        Model {
            id: Model::placeholder_id(name),
            name: name.to_string(),
            model_type: ModelType::Inference,
            provider: Provider::Ollama,
            model_path: ResourceId::new(format!("ollama://localhost:11434/{}", name)),
            digest: format!("sha256:test-digest-{}", name),
        }
    }

    #[test]
    fn save_and_load_model() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();
        let model = test_model("llama3");

        repo.save_model(&model).unwrap();

        let models = repo.load_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "llama3");
        assert_eq!(models[0].provider, Provider::Ollama);
    }

    #[test]
    fn delete_model() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();
        let model = test_model("llama3");

        repo.save_model(&model).unwrap();
        assert!(repo.model_exists("llama3").unwrap());

        let deleted = repo.delete_model("llama3").unwrap();
        assert!(deleted);
        assert!(!repo.model_exists("llama3").unwrap());
    }

    #[test]
    fn model_exists_check() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();

        assert!(!repo.model_exists("llama3").unwrap());

        repo.save_model(&test_model("llama3")).unwrap();
        assert!(repo.model_exists("llama3").unwrap());
    }

    #[test]
    fn upsert_replaces_existing() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();

        let mut model = test_model("llama3");
        repo.save_model(&model).unwrap();

        // Update engine type
        model.provider = Provider::OpenRouter;
        repo.save_model(&model).unwrap();

        let models = repo.load_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].provider, Provider::OpenRouter);
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
            public_key: None,
            object_storage: None,
            vector_storage: None,
            requirements: Requirements {
                models: HashMap::new(),
                services: HashMap::new(),
                mounts: HashMap::new(),
            },
            prompts: None,
        }
    }

    fn full_test_agent() -> Agent {
        use vlinder_core::domain::{Protocol, Provider, ServiceConfig, ServiceType};
        let mut models = HashMap::new();
        models.insert("phi3".to_string(), "phi3:latest".to_string());

        let mut services = HashMap::new();
        services.insert(
            ServiceType::Infer,
            ServiceConfig {
                provider: Provider::Ollama,
                protocol: Protocol::OpenAi,
                models: vec!["phi3:latest".to_string()],
            },
        );

        Agent {
            id: Agent::placeholder_id("full"),
            name: "full".to_string(),
            description: "Full agent".to_string(),
            source: Some("https://example.com".to_string()),
            runtime: RuntimeType::Container,
            executable: "localhost/full:latest".to_string(),
            image_digest: Some(ImageDigest::parse("sha256:abc123").unwrap()),
            public_key: None,
            object_storage: Some(ResourceId::new("sqlite:///data/objects.db")),
            vector_storage: Some(ResourceId::new("sqlite:///data/vectors.db")),
            requirements: Requirements {
                models,
                services,
                mounts: HashMap::new(),
            },
            prompts: Some(Prompts {
                intent_recognition: Some("Classify".to_string()),
                query_expansion: None,
                answer_generation: None,
                map_summarize: None,
                reduce_summaries: None,
                direct_summarize: None,
            }),
        }
    }

    #[test]
    fn save_and_load_agent() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();
        let agent = test_agent("echo");

        repo.save_agent(&agent).unwrap();

        let agents = repo.load_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "echo");
        assert_eq!(agents[0].runtime, RuntimeType::Container);
    }

    #[test]
    fn delete_agent() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();
        let agent = test_agent("echo");

        repo.save_agent(&agent).unwrap();
        assert!(repo.agent_exists("echo").unwrap());

        let deleted = repo.delete_agent("echo").unwrap();
        assert!(deleted);
        assert!(!repo.agent_exists("echo").unwrap());
    }

    #[test]
    fn agent_exists_check() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();

        assert!(!repo.agent_exists("echo").unwrap());

        repo.save_agent(&test_agent("echo")).unwrap();
        assert!(repo.agent_exists("echo").unwrap());
    }

    #[test]
    fn agent_upsert_replaces() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();

        let mut agent = test_agent("echo");
        repo.save_agent(&agent).unwrap();

        agent.description = "Updated description".to_string();
        repo.save_agent(&agent).unwrap();

        let agents = repo.load_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].description, "Updated description");
    }

    #[test]
    fn full_agent_round_trip() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();
        let agent = full_test_agent();

        repo.save_agent(&agent).unwrap();
        let agents = repo.load_agents().unwrap();
        assert_eq!(agents.len(), 1);

        let restored = &agents[0];
        assert_eq!(restored.name, "full");
        assert_eq!(restored.source.as_deref(), Some("https://example.com"));
        assert_eq!(
            restored.image_digest.as_ref().map(|d| d.as_str()),
            Some("sha256:abc123")
        );
        assert_eq!(
            restored.object_storage.as_ref().map(|r| r.as_str()),
            Some("sqlite:///data/objects.db")
        );
        assert_eq!(
            restored.vector_storage.as_ref().map(|r| r.as_str()),
            Some("sqlite:///data/vectors.db")
        );
        assert_eq!(
            restored.requirements.models.get("phi3").map(|s| s.as_str()),
            Some("phi3:latest")
        );
        assert_eq!(restored.requirements.services.len(), 1);
        let infer = &restored.requirements.services[&vlinder_core::domain::ServiceType::Infer];
        assert_eq!(infer.provider, vlinder_core::domain::Provider::Ollama);
        assert_eq!(infer.protocol, vlinder_core::domain::Protocol::OpenAi);
        assert_eq!(infer.models, vec!["phi3:latest"]);
        assert!(restored
            .prompts
            .as_ref()
            .unwrap()
            .intent_recognition
            .is_some());
    }

    #[test]
    fn agent_with_public_key_persists() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();
        let mut agent = test_agent("keyed");
        let key_bytes: Vec<u8> = (0..32).collect();
        agent.public_key = Some(key_bytes.clone());

        repo.save_agent(&agent).unwrap();
        let agents = repo.load_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].public_key.as_ref().unwrap(), &key_bytes);
    }

    #[test]
    fn agent_without_public_key_persists_none() {
        let repo = SqliteRegistryRepository::in_memory().unwrap();
        let agent = test_agent("no-key");

        repo.save_agent(&agent).unwrap();
        let agents = repo.load_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert!(agents[0].public_key.is_none());
    }
}
