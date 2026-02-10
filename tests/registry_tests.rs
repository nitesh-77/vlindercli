//! Integration tests for Registry that require agent fixtures.

use std::path::{Path, PathBuf};

use vlindercli::domain::{Agent, EngineType, InMemoryRegistry, Model, ModelType, Registry, RegistrationError, ResourceId, RuntimeType};

const FIXTURES: &str = "tests/fixtures/agents";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

fn load_agent(name: &str) -> Agent {
    Agent::load(&fixture(name)).unwrap()
}

// ============================================================================
// Agent Registration Tests
// ============================================================================

#[test]
fn agent_registration() {
    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Container);

    // Agent not found initially
    let agent_id = registry.agent_id("echo-agent");
    assert!(registry.get_agent(&agent_id).is_none());

    // Register agent — registry assigns identity
    let agent = load_agent("echo-agent");
    registry.register_agent(agent).unwrap();

    // Now found by registry-assigned id
    let agent = registry.get_agent(&agent_id).unwrap();
    assert_eq!(agent.name, "echo-agent");
}

// ============================================================================
// Runtime Selection Tests
// ============================================================================

#[test]
fn select_runtime_identifies_container_agent() {
    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Container);

    let agent = load_agent("echo-agent");

    // Agent declares runtime = "container" → Container runtime selected
    assert_eq!(registry.select_runtime(&agent), Some(RuntimeType::Container));
}

#[test]
fn select_runtime_returns_none_without_registered_runtime() {
    let registry = InMemoryRegistry::new(); // No runtimes registered

    let agent = load_agent("echo-agent");

    // No Container runtime available
    assert_eq!(registry.select_runtime(&agent), None);
}

// ============================================================================
// Model Deletion Tests
// ============================================================================

fn register_model_test_agent(registry: &InMemoryRegistry) {
    // model-test-agent requires phi3 and nomic-embed with these exact URIs
    let phi3 = Model {
        id: Model::placeholder_id("phi3"),
        name: "phi3".to_string(),
        model_type: ModelType::Inference,
        engine: EngineType::Ollama,
        model_path: ResourceId::new("http://127.0.0.1:9000/models/phi3"),
        digest: String::new(),
    };
    let nomic = Model {
        id: Model::placeholder_id("nomic-embed"),
        name: "nomic-embed".to_string(),
        model_type: ModelType::Embedding,
        engine: EngineType::Ollama,
        model_path: ResourceId::new("http://127.0.0.1:9000/models/nomic-embed"),
        digest: String::new(),
    };

    registry.register_runtime(RuntimeType::Container);
    registry.register_inference_engine(EngineType::Ollama);
    registry.register_embedding_engine(EngineType::Ollama);
    registry.register_model(phi3).unwrap();
    registry.register_model(nomic).unwrap();
    registry.register_agent(load_agent("model-test-agent")).unwrap();
}

#[test]
fn delete_model_blocked_by_deployed_agent() {
    let registry = InMemoryRegistry::new();
    register_model_test_agent(&registry);

    let result = registry.delete_model("phi3");
    assert!(result.is_err());

    let err = result.unwrap_err();
    match err {
        RegistrationError::ModelInUse(name, agents) => {
            assert_eq!(name, "phi3");
            assert!(agents.contains(&"model-test-agent".to_string()));
        }
        other => panic!("expected ModelInUse, got: {}", other),
    }

    // Model still exists
    assert!(registry.get_model("phi3").is_some());
}

#[test]
fn delete_model_succeeds_without_dependent_agents() {
    let registry = InMemoryRegistry::new();
    registry.register_inference_engine(EngineType::Ollama);

    let model = Model {
        id: Model::placeholder_id("unused"),
        name: "unused".to_string(),
        model_type: ModelType::Inference,
        engine: EngineType::Ollama,
        model_path: ResourceId::new("ollama://localhost:11434/unused"),
        digest: String::new(),
    };
    registry.register_model(model).unwrap();

    let deleted = registry.delete_model("unused").unwrap();
    assert!(deleted);
    assert!(registry.get_model("unused").is_none());
}

// ============================================================================
// Engine Availability Tests
// ============================================================================

#[test]
fn register_model_rejected_without_inference_engine() {
    let registry = InMemoryRegistry::new();
    // No inference engine registered

    let model = Model {
        id: Model::placeholder_id("phi3"),
        name: "phi3".to_string(),
        model_type: ModelType::Inference,
        engine: EngineType::Ollama,
        model_path: ResourceId::new("http://127.0.0.1:9000/models/phi3"),
        digest: String::new(),
    };

    let result = registry.register_model(model);
    assert!(result.is_err());
    match result.unwrap_err() {
        RegistrationError::InferenceEngineUnavailable(engine, model) => {
            assert_eq!(engine, EngineType::Ollama);
            assert_eq!(model, "phi3");
        }
        other => panic!("expected InferenceEngineUnavailable, got: {}", other),
    }
}

#[test]
fn register_model_rejected_without_embedding_engine() {
    let registry = InMemoryRegistry::new();
    // No embedding engine registered

    let model = Model {
        id: Model::placeholder_id("nomic-embed"),
        name: "nomic-embed".to_string(),
        model_type: ModelType::Embedding,
        engine: EngineType::Ollama,
        model_path: ResourceId::new("http://127.0.0.1:9000/models/nomic-embed"),
        digest: String::new(),
    };

    let result = registry.register_model(model);
    assert!(result.is_err());
    match result.unwrap_err() {
        RegistrationError::EmbeddingEngineUnavailable(engine, model) => {
            assert_eq!(engine, EngineType::Ollama);
            assert_eq!(model, "nomic-embed");
        }
        other => panic!("expected EmbeddingEngineUnavailable, got: {}", other),
    }
}
