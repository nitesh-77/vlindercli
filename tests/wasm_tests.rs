use std::path::{Path, PathBuf};
use vlindercli::domain::{Agent, AgentManifest, Daemon};

const FIXTURES: &str = "tests/fixtures/agents";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// Resolve manifest paths and return TOML string
fn load_resolved_manifest(agent_path: &Path) -> String {
    let manifest_path = agent_path.join("agent.toml");
    let manifest = AgentManifest::load(&manifest_path).unwrap();

    // Rebuild TOML with resolved paths
    let mut result = format!("name = \"{}\"\n", manifest.name);

    if manifest.description.contains('\n') {
        result.push_str(&format!("description = \"\"\"\n{}\"\"\"\n", manifest.description));
    } else {
        result.push_str(&format!("description = \"{}\"\n", manifest.description));
    }

    result.push_str(&format!("id = \"{}\"\n", manifest.id));

    if let Some(ref source) = manifest.source {
        result.push_str(&format!("source = \"{}\"\n", source));
    }

    result.push_str("\n[requirements]\n");
    result.push_str(&format!("services = {:?}\n", manifest.requirements.services));

    if !manifest.requirements.models.is_empty() {
        result.push_str("\n[requirements.models]\n");
        for (name, uri) in &manifest.requirements.models {
            result.push_str(&format!("{} = \"{}\"\n", name, uri));
        }
    }

    for mount in &manifest.mounts {
        result.push_str(&format!(
            "\n[[mounts]]\nhost_path = \"{}\"\nguest_path = \"{}\"\nmode = \"{}\"\n",
            mount.host_path, mount.guest_path, mount.mode
        ));
    }

    result
}

fn run_agent(name: &str, input: &str) -> String {
    let mut daemon = Daemon::new();
    let manifest_toml = load_resolved_manifest(&fixture(name));

    let job_id = daemon.invoke(&manifest_toml, input).unwrap();

    loop {
        daemon.tick();
        if let Some(result) = daemon.poll(&job_id) {
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
#[test]
fn pensieve_agent_fetches_and_summarizes() {
    let result = run_agent("pensieve", "https://httpbin.org/html");

    // Verify output contains the formatted sections from the agent
    assert!(result.contains("Source:"), "Expected 'Source:' in output");
    assert!(result.contains("Summary:"), "Expected 'Summary:' in output");
}
