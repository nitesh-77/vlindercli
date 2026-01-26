use std::path::{Path, PathBuf};
use vlindercli::domain::{Agent, AgentManifest};

const AGENT_FIXTURES: &str = "tests/fixtures/agents";
const MANIFEST_FIXTURES: &str = "tests/fixtures/manifests";

fn agent_fixture(name: &str) -> PathBuf {
    Path::new(AGENT_FIXTURES).join(name)
}

fn manifest_fixture(name: &str) -> PathBuf {
    Path::new(MANIFEST_FIXTURES).join(name).join("agent.toml")
}

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

// ============================================================================
// AgentManifest Tests
// ============================================================================

#[test]
fn manifest_load_parses_required_fields() {
    let (manifest, _raw) = AgentManifest::load(&manifest_fixture("minimal-manifest")).unwrap();

    assert_eq!(manifest.name, "minimal");
    assert_eq!(manifest.description, "Minimal valid manifest");
    assert_eq!(manifest.code, "agent.wasm");
}

#[test]
fn manifest_load_parses_optional_source() {
    let path = agent_fixture("pensieve").join("agent.toml");
    let (manifest, _raw) = AgentManifest::load(&path).unwrap();

    assert_eq!(manifest.source, Some("https://github.com/vlindercli/pensieve".to_string()));
}

#[test]
fn manifest_load_parses_requirements() {
    let path = agent_fixture("pensieve").join("agent.toml");
    let (manifest, _raw) = AgentManifest::load(&path).unwrap();

    assert!(manifest.requirements.models.contains(&"phi3".to_string()));
    assert!(manifest.requirements.models.contains(&"nomic-embed".to_string()));
    assert!(manifest.requirements.services.contains(&"infer".to_string()));
}

#[test]
fn manifest_load_parses_mounts() {
    let path = agent_fixture("mount-test-agent").join("agent.toml");
    let (manifest, _raw) = AgentManifest::load(&path).unwrap();

    assert_eq!(manifest.mounts.len(), 2);
    assert_eq!(manifest.mounts[0].host_path, "data");
    assert_eq!(manifest.mounts[0].guest_path, "/data");
    assert_eq!(manifest.mounts[0].mode, "ro");
}

#[test]
fn manifest_load_defaults_empty_optional_fields() {
    let (manifest, _raw) = AgentManifest::load(&manifest_fixture("minimal-manifest")).unwrap();

    assert!(manifest.source.is_none());
    assert!(manifest.prompts.is_none());
    assert!(manifest.mounts.is_empty());
}

#[test]
fn manifest_load_fails_for_invalid_toml() {
    let result = AgentManifest::load(&manifest_fixture("invalid-toml"));

    assert!(result.is_err());
}

#[test]
fn manifest_load_fails_for_missing_required_field() {
    let result = AgentManifest::load(&manifest_fixture("missing-required"));

    assert!(result.is_err());
}

#[test]
fn manifest_load_returns_raw_content() {
    let path = agent_fixture("echo-agent").join("agent.toml");
    let (_manifest, raw) = AgentManifest::load(&path).unwrap();

    assert!(raw.contains("echo-agent"));
    assert!(raw.contains("Test agent that echoes input"));
}
