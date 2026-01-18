use crate::domain::Agent;
use crate::inference::load_engine;
use extism::{Function, UserData};

pub struct Runtime;

impl Runtime {
    pub fn new() -> Self {
        Runtime
    }

    pub fn execute(&self, agent: &Agent, input: &str) -> String {
        agent.execute_with_functions(input, [make_infer_function(agent.clone())])
    }
}

fn make_infer_function(agent: Agent) -> Function {
    Function::new(
        "infer",
        [extism::PTR, extism::PTR],  // model_name, prompt
        [extism::PTR],
        UserData::new(agent),        // attach agent for validation
        |plugin, inputs, outputs, user_data| {
            let model_name: String = plugin.memory_get_val(&inputs[0])?;
            let prompt: String = plugin.memory_get_val(&inputs[1])?;

            let agent = user_data.get().unwrap();
            let agent = agent.lock().unwrap();

            let response = if !agent.has_model(&model_name) {
                format!("[error] model '{}' not declared by agent", model_name)
            } else {
                match load_engine(&model_name) {
                    Ok(engine) => engine.infer(&prompt, 512)
                        .unwrap_or_else(|e| format!("[error] {}", e)),
                    Err(e) => format!("[error] failed to load '{}': {}", model_name, e),
                }
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

