//! Model and ModelManifest tests.

use vlindercli::domain::{Model, ModelManifest, ModelType, ModelTypeConfig, ModelEngine, ModelEngineConfig};

// ============================================================================
// ModelManifest Tests (inline TOML - no fixtures needed)
// ============================================================================

fn parse_model_manifest(toml: &str) -> Result<ModelManifest, toml::de::Error> {
    toml::from_str(toml)
}

#[test]
fn model_manifest_parses_inference_type() {
    let manifest: ModelManifest = parse_model_manifest(r#"
        name = "phi3"
        type = "inference"
        engine = "llama"
        id = "file://./phi3.gguf"
    "#).unwrap();

    assert_eq!(manifest.name, "phi3");
    assert_eq!(manifest.model_type, ModelTypeConfig::Inference);
    assert_eq!(manifest.engine, ModelEngineConfig::Llama);
    assert_eq!(manifest.id, "file://./phi3.gguf");
}

#[test]
fn model_manifest_parses_embedding_type() {
    let manifest: ModelManifest = parse_model_manifest(r#"
        name = "nomic-embed"
        type = "embedding"
        engine = "llama"
        id = "file://./nomic.gguf"
    "#).unwrap();

    assert_eq!(manifest.name, "nomic-embed");
    assert_eq!(manifest.model_type, ModelTypeConfig::Embedding);
    assert_eq!(manifest.engine, ModelEngineConfig::Llama);
}

#[test]
fn model_manifest_fails_for_invalmodel_type() {
    let result = parse_model_manifest(r#"
        name = "bad"
        type = "unknown"
        engine = "llama"
        id = "file://./model.gguf"
    "#);
    assert!(result.is_err());
}

#[test]
fn model_manifest_fails_for_missing_field() {
    let result = parse_model_manifest(r#"
        name = "incomplete"
        type = "inference"
        engine = "llama"
    "#);
    assert!(result.is_err());
}

// ============================================================================
// Model Tests (need fixtures for model files)
// ============================================================================

#[test]
fn model_load_parses_manifest() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-model-load");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Create a dummy model file
    std::fs::write(temp_dir.join("phi3.gguf"), b"dummy").unwrap();

    let manifest = r#"
        name = "phi3"
        type = "inference"
        engine = "llama"
        id = "file://./phi3.gguf"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.name, "phi3");
    assert_eq!(model.model_type, ModelType::Inference);
    assert_eq!(model.engine, ModelEngine::Llama);
    assert_eq!(model.id.scheme(), Some("file"));
    assert!(model.id.path().unwrap().ends_with("phi3.gguf"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn model_load_fails_for_missing_model_file() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-model-missing");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest = r#"
        name = "phi3"
        type = "inference"
        engine = "llama"
        id = "file://./nonexistent.gguf"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let result = Model::load(&temp_dir.join("model.toml"));
    assert!(result.is_err(), "Should fail when model file doesn't exist");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn model_type_from_manifest() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-model-types");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    std::fs::write(temp_dir.join("model.gguf"), b"dummy").unwrap();

    // Test inference type
    let inference_manifest = r#"
        name = "inference-model"
        type = "inference"
        engine = "llama"
        id = "file://./model.gguf"
    "#;
    std::fs::write(temp_dir.join("inference.toml"), inference_manifest).unwrap();
    let model = Model::load(&temp_dir.join("inference.toml")).unwrap();
    assert_eq!(model.model_type, ModelType::Inference);

    // Test embedding type
    let embedding_manifest = r#"
        name = "embedding-model"
        type = "embedding"
        engine = "llama"
        id = "file://./model.gguf"
    "#;
    std::fs::write(temp_dir.join("embedding.toml"), embedding_manifest).unwrap();
    let model = Model::load(&temp_dir.join("embedding.toml")).unwrap();
    assert_eq!(model.model_type, ModelType::Embedding);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// Model Type Helpers (useful for runtime validation)
// ============================================================================

#[test]
fn model_type_can_be_compared() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-model-compare");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("model.gguf"), b"dummy").unwrap();

    let manifest = r#"
        name = "test"
        type = "inference"
        engine = "llama"
        id = "file://./model.gguf"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();

    // Runtime can validate model type before calling inference/embedding
    assert!(model.model_type == ModelType::Inference);
    assert!(model.model_type != ModelType::Embedding);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn model_engine_is_llama() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-model-engine");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("model.gguf"), b"dummy").unwrap();

    let manifest = r#"
        name = "test"
        type = "inference"
        engine = "llama"
        id = "file://./model.gguf"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.engine, ModelEngine::Llama);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
