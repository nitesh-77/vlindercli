use std::path::PathBuf;
use vlindercli::domain::Agent;

#[test]
fn agent_load_parses_vlinderfile() {
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
fn agent_has_model_from_vlinderfile() {
    // Pensieve declares phi3 and nomic-embed in its Vlinderfile
    let agent = Agent::load("pensieve").unwrap();

    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3"));
}

#[test]
fn agent_default_mount_when_none_declared() {
    // echo-agent has no [[mounts]] section
    let agent = Agent::load("echo-agent").unwrap();

    // Should be empty in struct (no mounts declared)
    assert!(agent.mounts.is_empty());

    // resolve_mounts() should return default: mnt -> /
    // But only if mnt directory exists, so we need to create it first
    let mnt_path = vlindercli::config::agent_mnt_path("echo-agent");
    std::fs::create_dir_all(&mnt_path).unwrap();

    let resolved = agent.resolve_mounts();
    assert_eq!(resolved.len(), 1);

    let (host_path, guest_path) = &resolved[0];
    assert!(host_path.ends_with("echo-agent/mnt"), "host_path should end with echo-agent/mnt, got: {}", host_path);
    assert_eq!(guest_path, &PathBuf::from("/"));
    // Default mode is rw, so no "ro:" prefix
    assert!(!host_path.starts_with("ro:"));
}

#[test]
fn agent_explicit_mounts_from_vlinderfile() {
    // mount-test-agent has explicit [[mounts]] section
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
fn agent_resolve_mounts_skips_nonexistent_paths() {
    let agent = Agent::load("echo-agent").unwrap();

    // Remove mnt directory if it exists to test skipping behavior
    let mnt_path = vlindercli::config::agent_mnt_path("echo-agent");
    let _ = std::fs::remove_dir_all(&mnt_path);

    // resolve_mounts should return empty vec when path doesn't exist
    let resolved = agent.resolve_mounts();
    assert!(resolved.is_empty(), "Should skip non-existent mount paths");
}

#[test]
fn agent_resolve_mounts_ro_prefix() {
    // mount-test-agent has ro and rw mounts
    let agent = Agent::load("mount-test-agent").unwrap();

    assert_eq!(agent.mounts.len(), 2);

    // Create the mount directories
    let agent_dir = vlindercli::config::agent_dir("mount-test-agent");
    std::fs::create_dir_all(agent_dir.join("data")).unwrap();
    std::fs::create_dir_all(agent_dir.join("output")).unwrap();

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
