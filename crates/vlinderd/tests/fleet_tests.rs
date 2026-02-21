use std::path::{Path, PathBuf};
use vlinderd::domain::{Fleet, FleetManifest};

const FLEET_FIXTURES: &str = "tests/fixtures/fleets";

fn fleet_fixture(name: &str) -> PathBuf {
    Path::new(FLEET_FIXTURES).join(name)
}

// ============================================================================
// FleetManifest Tests (inline TOML - no fixtures needed)
// ============================================================================

fn parse_fleet_manifest(toml: &str) -> Result<FleetManifest, toml::de::Error> {
    toml::from_str(toml)
}

#[test]
fn manifest_parses_required_fields() {
    let manifest: FleetManifest = parse_fleet_manifest(r#"
        name = "test-fleet"
        entry = "main"

        [agents.main]
        path = "agents/main"
    "#).unwrap();

    assert_eq!(manifest.name, "test-fleet");
    assert_eq!(manifest.entry, "main");
    assert!(manifest.agents.contains_key("main"));
}

#[test]
fn manifest_parses_agent_paths() {
    let manifest: FleetManifest = parse_fleet_manifest(r#"
        name = "test-fleet"
        entry = "orchestrator"

        [agents.orchestrator]
        path = "agents/orchestrator"

        [agents.worker]
        path = "agents/worker"
    "#).unwrap();

    assert_eq!(manifest.agents.get("orchestrator").unwrap().path, "agents/orchestrator");
    assert_eq!(manifest.agents.get("worker").unwrap().path, "agents/worker");
}

#[test]
fn manifest_fails_for_invalid_toml() {
    let result = parse_fleet_manifest("this is not valid toml {{{{");
    assert!(result.is_err());
}

#[test]
fn manifest_fails_for_missing_required_field() {
    // Missing entry field
    let result = parse_fleet_manifest(r#"
        name = "incomplete"

        [agents.main]
        path = "agents/main"
    "#);
    assert!(result.is_err());
}

// ============================================================================
// Fleet Tests (need fixtures for directory structure validation)
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

#[test]
fn fleet_build_context_lists_non_entry_agents() {
    let fleet = Fleet::load(&fleet_fixture("test-fleet")).unwrap();
    let context = fleet.build_context().unwrap();

    // Context should mention the fleet name
    assert!(context.contains("test-fleet"), "context: {}", context);

    // Context should list "upper" (non-entry agent) but not "echo" (entry agent)
    assert!(context.contains("upper"), "context should list non-entry agent: {}", context);
    assert!(context.contains("uppercases"), "context should include agent description: {}", context);
    assert!(!context.contains("- echo:"), "context should NOT list entry agent: {}", context);
}

#[test]
fn fleet_agents_iterator() {
    let fleet = Fleet::load(&fleet_fixture("test-fleet")).unwrap();
    let agent_names: Vec<&str> = fleet.agents().map(|(name, _)| name).collect();
    assert!(agent_names.contains(&"echo"));
    assert!(agent_names.contains(&"upper"));
}
