//! Integration tests for loader functions that require fixtures.

use std::path::PathBuf;

use vlinderd::loader::{load_agent, load_fleet_manifest};

fn fixture_uri(name: &str) -> String {
    let path = PathBuf::from("tests/fixtures/agents")
        .join(name)
        .canonicalize()
        .unwrap();
    format!("file://{}", path.display())
}

fn fleet_fixture_uri(name: &str) -> String {
    let path = PathBuf::from("tests/fixtures/fleets")
        .join(name)
        .canonicalize()
        .unwrap();
    format!("file://{}", path.display())
}

#[test]
#[ignore] // Run via: just run-integration-tests
fn load_agent_with_file_uri() {
    let agent = load_agent(&fixture_uri("echo-agent")).unwrap();
    assert_eq!(agent.name, "echo-agent");
}

#[test]
#[ignore] // Run via: just run-integration-tests
fn load_fleet_manifest_with_file_uri() {
    let manifest = load_fleet_manifest(&fleet_fixture_uri("test-fleet")).unwrap();
    assert_eq!(manifest.name, "test-fleet");
}
