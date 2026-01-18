use vlindercli::domain::{Agent, Model};

#[test]
fn agent_load_derives_wasm_path() {
    let agent = Agent::load("echo-agent", vec![]).unwrap();
    assert_eq!(agent.name, "echo-agent");
}

#[test]
fn agent_load_fails_for_unknown() {
    let result = Agent::load("unknown-agent", vec![]);
    assert!(result.is_err());
}

#[test]
fn agent_has_model_validation() {
    let agent = Agent::load("echo-agent", vec![
        Model { name: "phi3".to_string() },
    ]).unwrap();

    assert!(agent.has_model("phi3"));
    assert!(!agent.has_model("llama3"));
}
