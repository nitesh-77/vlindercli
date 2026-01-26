use std::path::{Path, PathBuf};
use vlindercli::domain::{Agent, AgentManifest};

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
        models = []
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
        models = []
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
        models = ["phi3", "nomic-embed"]
        services = ["infer", "embed"]
    "#).unwrap();

    assert!(manifest.requirements.models.contains(&"phi3".to_string()));
    assert!(manifest.requirements.models.contains(&"nomic-embed".to_string()));
    assert!(manifest.requirements.services.contains(&"infer".to_string()));
}

#[test]
fn manifest_parses_mounts() {
    let manifest: AgentManifest = parse_manifest(r#"
        name = "test-agent"
        description = "A test agent"
        code = "agent.wasm"

        [requirements]
        models = []
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
        models = []
        services = []
    "#).unwrap();

    assert!(manifest.source.is_none());
    assert!(manifest.prompts.is_none());
    assert!(manifest.mounts.is_empty());
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
