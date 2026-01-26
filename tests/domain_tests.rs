use std::path::{Path, PathBuf};
use vlindercli::domain::{Agent, AgentManifest, Model, ModelManifest, ModelType, ModelTypeConfig, ModelEngine, ModelEngineConfig};

const AGENT_FIXTURES: &str = "tests/fixtures/agents";

fn agent_fixture(name: &str) -> PathBuf {
    Path::new(AGENT_FIXTURES).join(name)
}

// ============================================================================
// AgentManifest Tests (inline TOML - no fixtures needed)
// ============================================================================

fn parse_manifest(toml: &str) -> Result<AgentManifest, toml::de::Error> {
    toml::from_str(toml)
}

#[test]
fn manifest_parses_required_fields() {
    let manifest: AgentManifest = parse_manifest(r#"
        name = "test-agent"
        description = "A test agent"
        code = "agent.wasm"

        [requirements]
        services = []
    "#).unwrap();

    assert_eq!(manifest.name, "test-agent");
    assert_eq!(manifest.description, "A test agent");
    assert_eq!(manifest.code, "agent.wasm");
}

#[test]
fn manifest_parses_optional_source() {
    let manifest: AgentManifest = parse_manifest(r#"
        name = "test-agent"
        description = "A test agent"
        code = "agent.wasm"
        source = "https://github.com/example/agent"

        [requirements]
        services = []
    "#).unwrap();

    assert_eq!(manifest.source, Some("https://github.com/example/agent".to_string()));
}

#[test]
fn manifest_parses_requirements() {
    let manifest: AgentManifest = parse_manifest(r#"
        name = "test-agent"
        description = "A test agent"
        code = "agent.wasm"

        [requirements]
        services = ["infer", "embed"]

        [requirements.models]
        phi3 = "file://./models/phi3.toml"
        nomic-embed = "file://./models/nomic.toml"
    "#).unwrap();

    assert!(manifest.requirements.models.contains_key("phi3"));
    assert!(manifest.requirements.models.contains_key("nomic-embed"));
    assert!(manifest.requirements.services.contains(&"infer".to_string()));
}

#[test]
fn manifest_parses_mounts() {
    let manifest: AgentManifest = parse_manifest(r#"
        name = "test-agent"
        description = "A test agent"
        code = "agent.wasm"

        [requirements]
        services = []

        [[mounts]]
        host_path = "data"
        guest_path = "/data"
        mode = "ro"

        [[mounts]]
        host_path = "output"
        guest_path = "/output"
        mode = "rw"
    "#).unwrap();

    assert_eq!(manifest.mounts.len(), 2);
    assert_eq!(manifest.mounts[0].host_path, "data");
    assert_eq!(manifest.mounts[0].guest_path, "/data");
    assert_eq!(manifest.mounts[0].mode, "ro");
    assert_eq!(manifest.mounts[1].mode, "rw");
}

#[test]
fn manifest_defaults_empty_optional_fields() {
    let manifest: AgentManifest = parse_manifest(r#"
        name = "minimal"
        description = "Minimal valid manifest"
        code = "agent.wasm"

        [requirements]
        services = []
    "#).unwrap();

    assert!(manifest.source.is_none());
    assert!(manifest.prompts.is_none());
    assert!(manifest.mounts.is_empty());
    assert!(manifest.requirements.models.is_empty());
}

#[test]
fn manifest_fails_for_invalid_toml() {
    let result = parse_manifest("this is not valid toml {{{{");
    assert!(result.is_err());
}

#[test]
fn manifest_fails_for_missing_required_field() {
    let result = parse_manifest(r#"
        name = "incomplete"
        description = "Missing code field"

        [requirements]
        models = []
        services = []
    "#);
    assert!(result.is_err());
}

// ============================================================================
// Agent Tests (need fixtures for WASM files and directory structure)
// ============================================================================

#[test]
fn agent_load_parses_manifest() {
    let agent = Agent::load(&agent_fixture("echo-agent")).unwrap();
    assert_eq!(agent.name, "echo-agent");
    assert_eq!(agent.description, "Test agent that echoes input");
}

#[test]
fn agent_load_fails_for_unknown() {
    let result = Agent::load(Path::new("nonexistent-agent"));
    assert!(result.is_err());
}

#[test]
fn agent_has_model_from_manifest() {
    let agent = Agent::load(&agent_fixture("pensieve")).unwrap();

    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3"));
}

#[test]
fn agent_no_mounts_when_none_declared() {
    let agent = Agent::load(&agent_fixture("echo-agent")).unwrap();

    // No mounts declared → no filesystem access (ADR 019)
    assert!(agent.mounts.is_empty());
}

#[test]
fn agent_explicit_mounts_from_manifest() {
    let agent = Agent::load(&agent_fixture("mount-test-agent")).unwrap();

    assert_eq!(agent.mounts.len(), 2);

    // Mounts are resolved to absolute paths
    assert!(agent.mounts[0].host_path.ends_with("data"));
    assert_eq!(agent.mounts[0].guest_path, PathBuf::from("/data"));
    assert!(agent.mounts[0].readonly);

    assert!(agent.mounts[1].host_path.ends_with("output"));
    assert_eq!(agent.mounts[1].guest_path, PathBuf::from("/output"));
    assert!(!agent.mounts[1].readonly);
}

#[test]
fn agent_load_fails_for_missing_mount() {
    let result = Agent::load(&agent_fixture("missing-mount-agent"));
    assert!(result.is_err(), "Should fail when mount path doesn't exist");
}

#[test]
fn agent_dir_set_from_load_path() {
    let agent = Agent::load(&agent_fixture("echo-agent")).unwrap();
    assert!(agent.agent_dir.ends_with("echo-agent"));
}

#[test]
fn agent_db_path() {
    let agent = Agent::load(&agent_fixture("echo-agent")).unwrap();
    assert!(agent.db_path().ends_with("agent.db"));
}

#[test]
fn agent_code_resolved_to_uri() {
    let agent = Agent::load(&agent_fixture("echo-agent")).unwrap();

    assert!(agent.code.starts_with("file://"));
    assert!(agent.code.ends_with(".wasm"));
}

#[test]
fn agent_load_fails_for_missing_code() {
    // Create temp directory with manifest pointing to non-existent code
    let temp_dir = std::env::temp_dir().join("vlinder-test-missing-code");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest = r#"
        name = "missing-code-agent"
        description = "Agent with missing code"
        code = "nonexistent.wasm"

        [requirements]
        models = []
        services = []
    "#;
    std::fs::write(temp_dir.join("agent.toml"), manifest).unwrap();

    let result = Agent::load(&temp_dir);
    assert!(result.is_err(), "Should fail when code file doesn't exist");

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_dir);
}

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
        model = "file://./phi3.gguf"
    "#).unwrap();

    assert_eq!(manifest.name, "phi3");
    assert_eq!(manifest.model_type, ModelTypeConfig::Inference);
    assert_eq!(manifest.engine, ModelEngineConfig::Llama);
    assert_eq!(manifest.model, "file://./phi3.gguf");
}

#[test]
fn model_manifest_parses_embedding_type() {
    let manifest: ModelManifest = parse_model_manifest(r#"
        name = "nomic-embed"
        type = "embedding"
        engine = "llama"
        model = "file://./nomic.gguf"
    "#).unwrap();

    assert_eq!(manifest.name, "nomic-embed");
    assert_eq!(manifest.model_type, ModelTypeConfig::Embedding);
    assert_eq!(manifest.engine, ModelEngineConfig::Llama);
}

#[test]
fn model_manifest_fails_for_invalid_type() {
    let result = parse_model_manifest(r#"
        name = "bad"
        type = "unknown"
        engine = "llama"
        model = "file://./model.gguf"
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
        model = "file://./phi3.gguf"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.name, "phi3");
    assert_eq!(model.model_type, ModelType::Inference);
    assert_eq!(model.engine, ModelEngine::Llama);
    assert!(model.model.starts_with("file://"));
    assert!(model.model.ends_with("phi3.gguf"));

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
        model = "file://./nonexistent.gguf"
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
        model = "file://./model.gguf"
    "#;
    std::fs::write(temp_dir.join("inference.toml"), inference_manifest).unwrap();
    let model = Model::load(&temp_dir.join("inference.toml")).unwrap();
    assert_eq!(model.model_type, ModelType::Inference);

    // Test embedding type
    let embedding_manifest = r#"
        name = "embedding-model"
        type = "embedding"
        engine = "llama"
        model = "file://./model.gguf"
    "#;
    std::fs::write(temp_dir.join("embedding.toml"), embedding_manifest).unwrap();
    let model = Model::load(&temp_dir.join("embedding.toml")).unwrap();
    assert_eq!(model.model_type, ModelType::Embedding);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// Agent + Model Integration Tests
// ============================================================================
// Demonstrates the pattern: Agent stores model URIs, runtime resolves them.

#[test]
fn agent_with_model_uris() {
    // Load agent - stores name→URI map in requirements.models
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // Agent has model by name (key in the map)
    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3")); // Not declared

    // Can get the URI for a model
    assert_eq!(
        agent.model_uri("phi3"),
        Some("file://./models/inference.toml")
    );
}

#[test]
fn runtime_resolves_model_uris() {
    use vlindercli::loader;

    // Load agent
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // Runtime resolves model URIs when needed (name → uri map)
    for (model_name, model_uri) in &agent.requirements.models {
        // Resolve relative URI against agent_dir
        let resolved_uri = resolve_model_uri(model_uri, &agent.agent_dir);
        let model = loader::load_model(&resolved_uri).unwrap();

        // Verify model name matches key and type is correct
        assert_eq!(&model.name, model_name);
        match model.name.as_str() {
            "phi3" => assert_eq!(model.model_type, ModelType::Inference),
            "nomic-embed" => assert_eq!(model.model_type, ModelType::Embedding),
            name => panic!("unexpected model: {}", name),
        }
    }
}

/// Helper to resolve file:// URIs with relative paths against a base directory.
/// In production, this would live in the runtime or loader module.
fn resolve_model_uri(uri: &str, base_dir: &Path) -> String {
    if let Some(path) = uri.strip_prefix("file://") {
        if path.starts_with("./") || !Path::new(path).is_absolute() {
            // Relative path - resolve against base_dir
            let resolved = base_dir.join(path.strip_prefix("./").unwrap_or(path));
            return format!("file://{}", resolved.display());
        }
    }
    uri.to_string()
}

#[test]
fn agent_with_empty_models_list() {
    // echo-agent has no model requirements
    let agent = Agent::load(&agent_fixture("echo-agent")).unwrap();
    assert!(agent.requirements.models.is_empty());
}

#[test]
fn resolve_model_uri_absolute_path() {
    let base = Path::new("/some/agent/dir");

    // Absolute URI should pass through unchanged
    let uri = "file:///absolute/path/to/model.toml";
    assert_eq!(resolve_model_uri(uri, base), uri);
}

#[test]
fn resolve_model_uri_relative_with_dot_slash() {
    let base = Path::new("/agent/dir");

    let uri = "file://./models/phi3.toml";
    let resolved = resolve_model_uri(uri, base);
    assert_eq!(resolved, "file:///agent/dir/models/phi3.toml");
}

#[test]
fn resolve_model_uri_relative_without_dot_slash() {
    let base = Path::new("/agent/dir");

    let uri = "file://models/phi3.toml";
    let resolved = resolve_model_uri(uri, base);
    assert_eq!(resolved, "file:///agent/dir/models/phi3.toml");
}

#[test]
fn resolve_model_uri_non_file_scheme_unchanged() {
    let base = Path::new("/agent/dir");

    // Non-file:// URIs pass through (even if invalid for our loader)
    let uri = "ollama://phi3";
    assert_eq!(resolve_model_uri(uri, base), uri);
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
        model = "file://./model.gguf"
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
    use vlindercli::domain::ModelEngine;

    let temp_dir = std::env::temp_dir().join("vlinder-test-model-engine");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("model.gguf"), b"dummy").unwrap();

    let manifest = r#"
        name = "test"
        type = "inference"
        engine = "llama"
        model = "file://./model.gguf"
    "#;
    std::fs::write(temp_dir.join("model.toml"), manifest).unwrap();

    let model = Model::load(&temp_dir.join("model.toml")).unwrap();
    assert_eq!(model.engine, ModelEngine::Llama);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// Agent.model_uri() Tests
// ============================================================================

#[test]
fn model_uri_returns_some_for_declared() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    assert_eq!(
        agent.model_uri("phi3"),
        Some("file://./models/inference.toml")
    );
    assert_eq!(
        agent.model_uri("nomic-embed"),
        Some("file://./models/embedding.toml")
    );
}

#[test]
fn model_uri_returns_none_for_undeclared() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    assert_eq!(agent.model_uri("llama3"), None);
    assert_eq!(agent.model_uri("gpt-4"), None);
}
