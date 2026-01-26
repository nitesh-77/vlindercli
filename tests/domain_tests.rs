use std::path::{Path, PathBuf};
use vlindercli::domain::Agent;

const FIXTURES: &str = "tests/fixtures/agents";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

#[test]
fn agent_load_parses_manifest() {
    let agent = Agent::load(&fixture("echo-agent")).unwrap();
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
    let agent = Agent::load(&fixture("pensieve")).unwrap();

    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3"));
}

#[test]
fn agent_no_mounts_when_none_declared() {
    let agent = Agent::load(&fixture("echo-agent")).unwrap();

    // No mounts declared → no filesystem access (ADR 019)
    assert!(agent.mounts.is_empty());
    assert!(agent.resolve_mounts().is_empty());
}

#[test]
fn agent_explicit_mounts_from_manifest() {
    let agent = Agent::load(&fixture("mount-test-agent")).unwrap();

    assert_eq!(agent.mounts.len(), 2);

    assert_eq!(agent.mounts[0].host_path, "data");
    assert_eq!(agent.mounts[0].guest_path, "/data");
    assert_eq!(agent.mounts[0].mode, "ro");

    assert_eq!(agent.mounts[1].host_path, "output");
    assert_eq!(agent.mounts[1].guest_path, "/output");
    assert_eq!(agent.mounts[1].mode, "rw");
}

#[test]
fn agent_load_fails_for_missing_mount() {
    let result = Agent::load(&fixture("missing-mount-agent"));
    assert!(result.is_err(), "Should fail when mount path doesn't exist");
}

#[test]
fn agent_resolve_mounts_ro_prefix() {
    let agent = Agent::load(&fixture("mount-test-agent")).unwrap();

    let resolved = agent.resolve_mounts();
    assert_eq!(resolved.len(), 2);

    let ro_mount = resolved.iter().find(|(_, g)| g == &PathBuf::from("/data"));
    assert!(ro_mount.is_some());
    let (ro_host, _) = ro_mount.unwrap();
    assert!(ro_host.starts_with("ro:"), "Read-only mount should have ro: prefix");

    let rw_mount = resolved.iter().find(|(_, g)| g == &PathBuf::from("/output"));
    assert!(rw_mount.is_some());
    let (rw_host, _) = rw_mount.unwrap();
    assert!(!rw_host.starts_with("ro:"), "Read-write mount should NOT have ro: prefix");
}

#[test]
fn agent_dir_set_from_load_path() {
    let agent = Agent::load(&fixture("echo-agent")).unwrap();
    assert!(agent.agent_dir.ends_with("echo-agent"));
}

#[test]
fn agent_db_path() {
    let agent = Agent::load(&fixture("echo-agent")).unwrap();
    assert!(agent.db_path().ends_with("agent.db"));
}
