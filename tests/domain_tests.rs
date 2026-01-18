use vlindercli::domain::{Model, ModelType, Behavior};

#[test]
fn model_has_type_and_name() {
    let model = Model {
        model_type: ModelType::Inference,
        name: "llama3".to_string(),
    };
    assert_eq!(model.model_type, ModelType::Inference);
    assert_eq!(model.name, "llama3");
}

#[test]
fn behavior_has_system_prompt() {
    let behavior = Behavior {
        system_prompt: "You are helpful.".to_string(),
    };
    assert_eq!(behavior.system_prompt, "You are helpful.");
}
