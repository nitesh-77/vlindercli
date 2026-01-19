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
