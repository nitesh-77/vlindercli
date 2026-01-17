use vlindercli::domain::{Model, Behavior};
use vlindercli::runtime::Runtime;

fn test_model() -> Model {
    Model { path: "models/test.gguf".to_string() }
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
        test_model(),
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
        test_model(),
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
        test_model(),
        test_behavior(),
    );
    assert!(result.is_err());
}

#[test]
fn agent_has_name_model_and_behavior() {
    let runtime = Runtime::new();
    let agent = runtime.spawn_agent(
        "echo-agent",
        "agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm",
        Model { path: "models/llama.gguf".to_string() },
        Behavior { system_prompt: "Be concise.".to_string() },
    ).unwrap();

    assert_eq!(agent.name, "echo-agent");
    assert_eq!(agent.model.path, "models/llama.gguf");
    assert_eq!(agent.behavior.system_prompt, "Be concise.");
}
