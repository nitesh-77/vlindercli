//! Model and ModelManifest tests.

use vlindercli::domain::{Model, ModelManifest, ModelType, ModelTypeConfig, Provider};

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
        provider = "ollama"
        model_path = "ollama://localhost:11434/phi3"
    "#).unwrap();

    assert_eq!(manifest.name, Some("phi3".to_string()));
    assert_eq!(manifest.model_type, ModelTypeConfig::Inference);
    assert_eq!(manifest.provider, Provider::Ollama);
    assert_eq!(manifest.model_path, "ollama://localhost:11434/phi3");
}

#[test]
fn model_manifest_parses_embedding_type() {
    let manifest: ModelManifest = parse_model_manifest(r#"
        name = "nomic-embed"
        type = "embedding"
        provider = "ollama"
        model_path = "ollama://localhost:11434/nomic-embed-text"
    "#).unwrap();

    assert_eq!(manifest.name, Some("nomic-embed".to_string()));
    assert_eq!(manifest.model_type, ModelTypeConfig::Embedding);
    assert_eq!(manifest.provider, Provider::Ollama);
}

#[test]
fn model_manifest_parses_openrouter_engine() {
    let manifest: ModelManifest = parse_model_manifest(r#"
        name = "claude-sonnet"
        type = "inference"
        provider = "openrouter"
        model_path = "openrouter://anthropic/claude-sonnet-4-20250514"
    "#).unwrap();

    assert_eq!(manifest.name, Some("claude-sonnet".to_string()));
    assert_eq!(manifest.model_type, ModelTypeConfig::Inference);
    assert_eq!(manifest.provider, Provider::OpenRouter);
    assert_eq!(manifest.model_path, "openrouter://anthropic/claude-sonnet-4-20250514");
}

#[test]
fn openrouter_model_loads_with_correct_engine_type() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-openrouter-model");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest = r#"
        name = "claude-sonnet"
        type = "inference"
        provider = "openrouter"
        model_path = "openrouter://anthropic/claude-sonnet-4-20250514"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.name, "claude-sonnet");
    assert_eq!(model.pfname(), "anthropic/claude-sonnet-4-20250514");
    assert_eq!(model.provider, Provider::OpenRouter);
    assert_eq!(model.model_type, ModelType::Inference);
    assert_eq!(model.model_path.as_str(), "openrouter://anthropic/claude-sonnet-4-20250514");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn model_manifest_fails_for_invalmodel_type() {
    let result = parse_model_manifest(r#"
        name = "bad"
        type = "unknown"
        provider = "ollama"
        model_path = "ollama://localhost:11434/bad"
    "#);
    assert!(result.is_err());
}

#[test]
fn model_manifest_fails_for_missing_field() {
    let result = parse_model_manifest(r#"
        name = "incomplete"
        type = "inference"
        provider = "ollama"
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

    let manifest = r#"
        name = "phi3"
        type = "inference"
        provider = "ollama"
        model_path = "ollama://localhost:11434/phi3"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.name, "phi3");
    assert_eq!(model.model_type, ModelType::Inference);
    assert_eq!(model.provider, Provider::Ollama);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn model_type_from_manifest() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-model-types");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Test inference type
    let inference_manifest = r#"
        name = "inference-model"
        type = "inference"
        provider = "ollama"
        model_path = "ollama://localhost:11434/inference-model"
    "#;
    std::fs::write(temp_dir.join("inference.toml"), inference_manifest).unwrap();
    let model = Model::load(&temp_dir.join("inference.toml")).unwrap();
    assert_eq!(model.model_type, ModelType::Inference);

    // Test embedding type
    let embedding_manifest = r#"
        name = "embedding-model"
        type = "embedding"
        provider = "ollama"
        model_path = "ollama://localhost:11434/embedding-model"
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

    let manifest = r#"
        name = "test"
        type = "inference"
        provider = "ollama"
        model_path = "ollama://localhost:11434/test"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();

    // Runtime can validate model type before calling inference/embedding
    assert!(model.model_type == ModelType::Inference);
    assert!(model.model_type != ModelType::Embedding);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn model_engine_is_ollama() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-model-engine");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest = r#"
        name = "test"
        type = "inference"
        provider = "ollama"
        model_path = "ollama://localhost:11434/test"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.provider, Provider::Ollama);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// Name vs pfname (ADR 094)
// model.name = registry name (from manifest `name` field)
// model.pfname() = provider-friendly name (derived from model_path)
// ============================================================================

#[test]
fn name_from_manifest_pfname_from_path_ollama() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-name-ollama");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest = r#"
        name = "nomic-embed"
        type = "embedding"
        provider = "ollama"
        model_path = "ollama://localhost:11434/nomic-embed-text:latest"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.name, "nomic-embed");
    assert_eq!(model.pfname(), "nomic-embed-text:latest");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn name_from_manifest_pfname_from_path_openrouter() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-name-openrouter");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest = r#"
        name = "claude-sonnet"
        type = "inference"
        provider = "openrouter"
        model_path = "openrouter://anthropic/claude-sonnet-4"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.name, "claude-sonnet");
    assert_eq!(model.pfname(), "anthropic/claude-sonnet-4");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn name_and_pfname_when_manifest_name_matches_path() {
    let temp_dir = std::env::temp_dir().join("vlinder-test-name-match");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest = r#"
        name = "phi3:latest"
        type = "inference"
        provider = "ollama"
        model_path = "ollama://localhost:11434/phi3:latest"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.name, "phi3:latest");
    assert_eq!(model.pfname(), "phi3:latest");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn manifest_without_name_field_parses() {
    let manifest: ModelManifest = toml::from_str(r#"
        type = "inference"
        provider = "ollama"
        model_path = "ollama://localhost:11434/phi3"
    "#).unwrap();

    assert_eq!(manifest.name, None);
    assert_eq!(manifest.model_path, "ollama://localhost:11434/phi3");
}
