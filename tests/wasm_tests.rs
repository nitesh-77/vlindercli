use vlindercli::domain::{Agent, Model};
use vlindercli::runtime::Runtime;

#[test]
fn agent_echo() {
    let agent = Agent::load("echo-agent", vec![]).unwrap();
    let result = agent.execute("hello");
    assert_eq!(result, "echo: hello");
}

#[test]
fn agent_upper() {
    let agent = Agent::load("upper-agent", vec![]).unwrap();
    let result = agent.execute("hello");
    assert_eq!(result, "HELLO");
}

#[test]
fn load_fails_for_missing_agent() {
    let result = Agent::load("nonexistent-agent", vec![]);
    assert!(result.is_err());
}

#[test]
fn agent_has_name_and_models() {
    let agent = Agent::load("echo-agent", vec![
        Model { name: "phi3".to_string() },
        Model { name: "llama3".to_string() },
    ]).unwrap();
    assert_eq!(agent.name, "echo-agent");
    assert_eq!(agent.models.len(), 2);
    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("llama3"));
    assert!(!agent.has_model("unknown"));
}

/// Pensieve agent:
/// 1. Input: URL
/// 2. Fetches URL, strips HTML (pure Rust), caches both raw and clean text
/// 3. Chunks content and stores embeddings for semantic search
/// 4. Calls infer("phi3", "Summarize...") → runtime validates & runs inference
/// 5. Returns formatted output with stats, content preview, and summary
#[test]
fn pensieve_agent_fetches_and_summarizes() {
    let runtime = Runtime::new();
    let agent = Agent::load("pensieve", vec![
        Model { name: "phi3".to_string() },
        Model { name: "nomic-embed".to_string() },
    ]).unwrap();

    let result = runtime.execute(&agent, "https://httpbin.org/html");

    // Verify output contains the formatted sections from the agent
    assert!(result.contains("Source:"), "Expected 'Source:' in output");
    assert!(result.contains("Summary:"), "Expected 'Summary:' in output");
}
