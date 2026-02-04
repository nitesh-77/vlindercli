//! SQLite implementation of RegistryRepository.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::domain::{Model, RegistryRepository, RepositoryError, StoredModel};

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

        let conn = Connection::open(path)
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let repo = Self { conn: Mutex::new(conn) };
        repo.init_schema()?;
        Ok(repo)
    }

    /// Open an in-memory database (for testing).
    pub fn in_memory() -> Result<Self, RepositoryError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let repo = Self { conn: Mutex::new(conn) };
        repo.init_schema()?;
        Ok(repo)
    }

    fn init_schema(&self) -> Result<(), RepositoryError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS models (
                name TEXT PRIMARY KEY,
                model_type TEXT NOT NULL,
                engine TEXT NOT NULL,
                model_path TEXT NOT NULL,
                digest TEXT NOT NULL
            )",
            [],
        ).map_err(|e| RepositoryError::Database(e.to_string()))?;
        Ok(())
    }
}

impl RegistryRepository for SqliteRegistryRepository {
    fn save_model(&self, model: &Model) -> Result<(), RepositoryError> {
        let stored = StoredModel::from_model(model);
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO models (name, model_type, engine, model_path, digest)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            [&stored.name, &stored.model_type, &stored.engine, &stored.model_path, &stored.digest],
        ).map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(())
    }

    fn load_models(&self) -> Result<Vec<Model>, RepositoryError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT name, model_type, engine, model_path, digest FROM models")
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(StoredModel {
                    name: row.get(0)?,
                    model_type: row.get(1)?,
                    engine: row.get(2)?,
                    model_path: row.get(3)?,
                    digest: row.get(4)?,
                })
            })
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let mut models = Vec::new();
        for row in rows {
            let stored = row.map_err(|e| RepositoryError::Database(e.to_string()))?;
            models.push(stored.to_model()?);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EngineType, ModelType, ResourceId};

    fn test_model(name: &str) -> Model {
        Model {
            id: Model::placeholder_id(name),
            name: name.to_string(),
            model_type: ModelType::Inference,
            engine: EngineType::Ollama,
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
        assert_eq!(models[0].engine, EngineType::Ollama);
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
        model.engine = EngineType::Llama;
        repo.save_model(&model).unwrap();

        let models = repo.load_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].engine, EngineType::Llama);
    }
}
