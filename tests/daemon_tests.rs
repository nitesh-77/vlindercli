//! Integration tests for PersistentRegistry + disk.
//!
//! These manipulate VLINDER_DIR and must run with --test-threads=1
//! or be marked #[ignore].

use vlindercli::domain::{EngineType, Model, ModelType, PersistentRegistry, Registry, RegistryRepository, ResourceId};
use vlindercli::config::Config;
use vlindercli::storage::SqliteRegistryRepository;

use tempfile::TempDir;

#[test]
#[ignore] // Mutates VLINDER_DIR — not safe in parallel
fn loads_models_from_registry_db() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("registry.db");

    // Pre-populate the registry
    {
        let repo = SqliteRegistryRepository::open(&db_path).unwrap();
        let model = Model {
            id: Model::placeholder_id("test-model"),
            name: "test-model".to_string(),
            model_type: ModelType::Inference,
            engine: EngineType::Ollama,
            model_path: ResourceId::new("ollama://localhost:11434/test-model"),
            digest: "sha256:test-digest".to_string(),
        };
        repo.save_model(&model).unwrap();
    }

    std::env::set_var("VLINDER_DIR", temp_dir.path());
    let config = Config::load();
    let registry = PersistentRegistry::open(&db_path, &config).unwrap();
    std::env::remove_var("VLINDER_DIR");

    let model = registry.get_model("test-model");
    assert!(model.is_some(), "test-model should be loaded from registry.db");
    assert_eq!(model.unwrap().name, "test-model");
}
