use std::path::{Path, PathBuf};
use vlindercli::domain::{Agent, Daemon};

const FIXTURES: &str = "tests/fixtures/agents";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

fn run_agent(name: &str, input: &str) -> String {
    let mut daemon = Daemon::new();

    // Deploy agent (runtime discovers automatically)
    let agent_id = daemon.harness.deploy_from_path(&fixture(name)).unwrap();

    // Invoke
    let job_id = daemon.harness.invoke(&agent_id, input).unwrap();

    loop {
        daemon.tick();
        if let Some(result) = daemon.harness.poll(&job_id) {
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn agent_echo() {
    let result = run_agent("echo-agent", "hello");
    assert_eq!(result, "echo: hello");
}

#[test]
fn agent_upper() {
    let result = run_agent("upper-agent", "hello");
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
///
/// NOTE: Requires real GGUF model weights in tests/fixtures/agents/pensieve/models/weights/
/// Run with: cargo test --test wasm_tests pensieve -- --ignored
#[test]
#[ignore = "requires real GGUF model weights"]
fn pensieve_agent_fetches_and_summarizes() {
    let result = run_agent("pensieve", "https://httpbin.org/html");

    // Verify output contains the formatted sections from the agent
    assert!(result.contains("Source:"), "Expected 'Source:' in output");
    assert!(result.contains("Summary:"), "Expected 'Summary:' in output");
}
