use vlindercli::domain::{Model, ModelType, Behavior};
use vlindercli::runtime::Runtime;

fn test_models() -> Vec<Model> {
    vec![Model {
        model_type: ModelType::Inference,
        name: "phi3".to_string(),
    }]
}

fn test_behavior() -> Behavior {
    Behavior { system_prompt: "You are helpful.".to_string() }
}

#[test]
fn agent_echo() {
    let runtime = Runtime::new();
    let agent = runtime.spawn_agent(
        "echo-agent",
        "agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm",
        test_models(),
        test_behavior(),
    ).unwrap();

    let result = agent.execute("hello");
    assert_eq!(result, "echo: hello");
}

#[test]
fn agent_upper() {
    let runtime = Runtime::new();
    let agent = runtime.spawn_agent(
        "upper-agent",
        "agents/upper-agent/target/wasm32-unknown-unknown/release/upper_agent.wasm",
        test_models(),
        test_behavior(),
    ).unwrap();

    let result = agent.execute("hello");
    assert_eq!(result, "HELLO");
}

#[test]
fn spawn_fails_for_missing_wasm() {
    let runtime = Runtime::new();
    let result = runtime.spawn_agent(
        "missing",
        "does/not/exist.wasm",
        test_models(),
        test_behavior(),
    );
    assert!(result.is_err());
}

#[test]
fn agent_has_name_models_and_behavior() {
    let runtime = Runtime::new();
    let agent = runtime.spawn_agent(
        "echo-agent",
        "agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm",
        vec![
            Model { model_type: ModelType::Inference, name: "llama3".to_string() },
            Model { model_type: ModelType::Embedding, name: "nomic-embed".to_string() },
        ],
        Behavior { system_prompt: "Be concise.".to_string() },
    ).unwrap();

    assert_eq!(agent.name, "echo-agent");
    assert_eq!(agent.models.len(), 2);
    assert_eq!(agent.models[0].model_type, ModelType::Inference);
    assert_eq!(agent.models[0].name, "llama3");
    assert_eq!(agent.models[1].model_type, ModelType::Embedding);
    assert_eq!(agent.behavior.system_prompt, "Be concise.");
}

/// Reader agent:
/// 1. Input: URL
/// 2. Agent fetches URL directly (Extism HTTP)
/// 3. Calls infer("Extract...") → runtime returns "[inferred] ..."
/// 4. Wasm trims whitespace
/// 5. Calls infer("3 key takeaways...") → runtime returns "[inferred] ..."
/// 6. Returns formatted output
#[test]
fn reader_agent_fetches_and_infers() {
    let runtime = Runtime::new();
    let agent = runtime.spawn_agent(
        "reader-agent",
        "agents/reader-agent/target/wasm32-unknown-unknown/release/reader_agent.wasm",
        test_models(),
        test_behavior(),
    ).unwrap();

    let result = runtime.execute(&agent, "https://httpbin.org/html");

    // Verify output contains inferred content (runtime echoes prompts with [inferred] prefix)
    assert!(result.contains("[inferred]"));
}
