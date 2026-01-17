use vlindercli::domain::{Model, ModelType, Behavior};

#[test]
fn model_has_type_and_path() {
    let model = Model {
        model_type: ModelType::Inference,
        path: "models/llama-2-7b.gguf".to_string(),
    };
    assert_eq!(model.model_type, ModelType::Inference);
    assert_eq!(model.path, "models/llama-2-7b.gguf");
}

#[test]
fn behavior_has_system_prompt() {
    let behavior = Behavior {
        system_prompt: "You are helpful.".to_string(),
    };
    assert_eq!(behavior.system_prompt, "You are helpful.");
}
