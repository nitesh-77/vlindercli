use std::path::PathBuf;
use vlindercli::domain::Agent;

#[test]
fn agent_load_parses_manifest() {
    let agent = Agent::load("echo-agent").unwrap();
    assert_eq!(agent.name, "echo-agent");
    assert_eq!(agent.description, "Test agent that echoes input");
}

#[test]
fn agent_load_fails_for_unknown() {
    let result = Agent::load("unknown-agent");
    assert!(result.is_err());
}

#[test]
fn agent_has_model_from_manifest() {
    // Pensieve declares phi3 and nomic-embed in its manifest
    let agent = Agent::load("pensieve").unwrap();

    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3"));
}

#[test]
fn agent_no_mounts_when_none_declared() {
    // echo-agent has no [[mounts]] section
    let agent = Agent::load("echo-agent").unwrap();

    // No mounts declared → no filesystem access (ADR 019)
    assert!(agent.mounts.is_empty());
    assert!(agent.resolve_mounts().is_empty());
}

#[test]
fn agent_explicit_mounts_from_manifest() {
    // mount-test-agent has explicit [[mounts]] section
    // Mount directories are part of the fixture (data/, output/)
    let agent = Agent::load("mount-test-agent").unwrap();

    // Should have two mounts declared
    assert_eq!(agent.mounts.len(), 2);

    // First mount: data -> /data (ro)
    assert_eq!(agent.mounts[0].host_path, "data");
    assert_eq!(agent.mounts[0].guest_path, "/data");
    assert_eq!(agent.mounts[0].mode, "ro");

    // Second mount: output -> /output (rw)
    assert_eq!(agent.mounts[1].host_path, "output");
    assert_eq!(agent.mounts[1].guest_path, "/output");
    assert_eq!(agent.mounts[1].mode, "rw");
}

#[test]
fn agent_load_fails_for_missing_mount() {
    // missing-mount-agent declares a mount to a nonexistent directory
    let result = Agent::load("missing-mount-agent");
    assert!(result.is_err(), "Should fail when explicit mount path doesn't exist");
}

#[test]
fn agent_resolve_mounts_ro_prefix() {
    // mount-test-agent has ro and rw mounts
    // Mount directories are part of the fixture (data/, output/)
    let agent = Agent::load("mount-test-agent").unwrap();

    assert_eq!(agent.mounts.len(), 2);

    let resolved = agent.resolve_mounts();
    assert_eq!(resolved.len(), 2);

    // Find the ro mount (data -> /data)
    let ro_mount = resolved.iter().find(|(_, g)| g == &PathBuf::from("/data"));
    assert!(ro_mount.is_some(), "Should have /data mount");
    let (ro_host, _) = ro_mount.unwrap();
    assert!(ro_host.starts_with("ro:"), "Read-only mount should have ro: prefix, got: {}", ro_host);

    // Find the rw mount (output -> /output)
    let rw_mount = resolved.iter().find(|(_, g)| g == &PathBuf::from("/output"));
    assert!(rw_mount.is_some(), "Should have /output mount");
    let (rw_host, _) = rw_mount.unwrap();
    assert!(!rw_host.starts_with("ro:"), "Read-write mount should NOT have ro: prefix, got: {}", rw_host);
}
