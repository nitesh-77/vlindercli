use vlindercli::domain::{Model, Behavior};

#[test]
fn model_has_name() {
    let model = Model { name: "llama3".to_string() };
    assert_eq!(model.name, "llama3");
}

#[test]
fn behavior_has_system_prompt() {
    let behavior = Behavior {
        system_prompt: "You are helpful.".to_string(),
    };
    assert_eq!(behavior.system_prompt, "You are helpful.");
}
