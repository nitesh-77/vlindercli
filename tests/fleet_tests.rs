use std::path::{Path, PathBuf};
use vlindercli::domain::{Fleet, FleetManifest};

const FLEET_FIXTURES: &str = "tests/fixtures/fleets";
const MANIFEST_FIXTURES: &str = "tests/fixtures/fleet-manifests";

fn fleet_fixture(name: &str) -> PathBuf {
    Path::new(FLEET_FIXTURES).join(name)
}

fn manifest_fixture(name: &str) -> PathBuf {
    Path::new(MANIFEST_FIXTURES).join(name).join("fleet.toml")
}

// ============================================================================
// Fleet Tests
// ============================================================================

#[test]
fn fleet_load_parses_manifest() {
    let fleet = Fleet::load(&fleet_fixture("test-fleet")).unwrap();
    assert_eq!(fleet.name, "test-fleet");
    assert_eq!(fleet.entry, "echo");
}

#[test]
fn fleet_load_fails_for_missing_manifest() {
    let result = Fleet::load(Path::new("nonexistent-fleet"));
    assert!(result.is_err());
}

#[test]
fn fleet_load_fails_when_entry_not_in_agents() {
    let result = Fleet::load(&fleet_fixture("invalid-entry-fleet"));
    assert!(result.is_err());
}

#[test]
fn fleet_load_fails_when_agent_path_missing() {
    let result = Fleet::load(&fleet_fixture("missing-agent-fleet"));
    assert!(result.is_err());
}

#[test]
fn fleet_project_dir_set_from_load_path() {
    let fleet = Fleet::load(&fleet_fixture("test-fleet")).unwrap();
    assert!(fleet.project_dir.ends_with("test-fleet"));
}

#[test]
fn fleet_has_agent() {
    let fleet = Fleet::load(&fleet_fixture("test-fleet")).unwrap();
    assert!(fleet.has_agent("echo"));
    assert!(fleet.has_agent("upper"));
    assert!(!fleet.has_agent("nonexistent"));
}

#[test]
fn fleet_agent_path() {
    let fleet = Fleet::load(&fleet_fixture("test-fleet")).unwrap();
    let path = fleet.agent_path("echo").unwrap();
    assert!(path.ends_with("agents/echo-agent"));
}

// ============================================================================
// FleetManifest Tests
// ============================================================================

#[test]
fn manifest_load_parses_required_fields() {
    let manifest = FleetManifest::load(&manifest_fixture("minimal")).unwrap();

    assert_eq!(manifest.name, "minimal-fleet");
    assert_eq!(manifest.entry, "main");
    assert!(manifest.agents.contains_key("main"));
}

#[test]
fn manifest_load_parses_agent_paths() {
    let manifest = FleetManifest::load(&manifest_fixture("minimal")).unwrap();

    let agent = manifest.agents.get("main").unwrap();
    assert_eq!(agent.path, "agents/main");
}

#[test]
fn manifest_load_fails_for_invalid_toml() {
    let result = FleetManifest::load(&manifest_fixture("invalid-toml"));
    assert!(result.is_err());
}

#[test]
fn manifest_load_fails_when_entry_not_in_agents() {
    let result = FleetManifest::load(&manifest_fixture("missing-entry"));
    assert!(result.is_err());
}
