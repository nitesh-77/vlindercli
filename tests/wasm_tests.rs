use std::path::{Path, PathBuf};
use vlindercli::domain::Agent;
use vlindercli::runtime::Runtime;

const FIXTURES: &str = "tests/fixtures/agents";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

#[test]
fn agent_echo() {
    let runtime = Runtime::new();
    let result = runtime.execute(&fixture("echo-agent"), "hello");
    assert_eq!(result, "echo: hello");
}

#[test]
fn agent_upper() {
    let runtime = Runtime::new();
    let result = runtime.execute(&fixture("upper-agent"), "hello");
    assert_eq!(result, "HELLO");
}

#[test]
fn load_fails_for_missing_agent() {
    let result = Agent::load(Path::new("nonexistent-agent"));
    assert!(result.is_err());
}

#[test]
fn agent_loads_requirements_from_manifest() {
    let agent = Agent::load(&fixture("pensieve")).unwrap();
    assert_eq!(agent.name, "pensieve");
    assert_eq!(agent.requirements.models.len(), 2);
    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
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
    let result = runtime.execute(&fixture("pensieve"), "https://httpbin.org/html");

    // Verify output contains the formatted sections from the agent
    assert!(result.contains("Source:"), "Expected 'Source:' in output");
    assert!(result.contains("Summary:"), "Expected 'Summary:' in output");
}
