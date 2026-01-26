use std::path::{Path, PathBuf};
use vlindercli::domain::Fleet;

const FIXTURES: &str = "tests/fixtures/fleets";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

#[test]
fn fleet_load_parses_manifest() {
    let fleet = Fleet::load(&fixture("test-fleet")).unwrap();
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
    let result = Fleet::load(&fixture("invalid-entry-fleet"));
    assert!(result.is_err());
}

#[test]
fn fleet_load_fails_when_agent_path_missing() {
    let result = Fleet::load(&fixture("missing-agent-fleet"));
    assert!(result.is_err());
}

#[test]
fn fleet_project_dir_set_from_load_path() {
    let fleet = Fleet::load(&fixture("test-fleet")).unwrap();
    assert!(fleet.project_dir.ends_with("test-fleet"));
}

#[test]
fn fleet_has_agent() {
    let fleet = Fleet::load(&fixture("test-fleet")).unwrap();
    assert!(fleet.has_agent("echo"));
    assert!(fleet.has_agent("upper"));
    assert!(!fleet.has_agent("nonexistent"));
}

#[test]
fn fleet_agent_path() {
    let fleet = Fleet::load(&fixture("test-fleet")).unwrap();
    let path = fleet.agent_path("echo").unwrap();
    assert!(path.ends_with("agents/echo-agent"));
}
