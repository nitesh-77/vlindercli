use crate::domain::{Agent, Model, Behavior, LoadError};
use extism::{Function, UserData};

pub struct Runtime;

impl Runtime {
    pub fn new() -> Self {
        Runtime
    }

    pub fn spawn_agent(
        &self,
        name: &str,
        wasm_path: &str,
        models: Vec<Model>,
        behavior: Behavior,
    ) -> Result<Agent, LoadError> {
        Agent::load(name, wasm_path, models, behavior)
    }

    pub fn execute(&self, agent: &Agent, input: &str) -> String {
        agent.execute_with_functions(input, [make_infer_function()])
    }
}

fn make_infer_function() -> Function {
    Function::new(
        "infer",
        [extism::PTR],
        [extism::PTR],
        UserData::new(()),
        |plugin, inputs, outputs, _| {
            let prompt: String = plugin.memory_get_val(&inputs[0])?;

            // TODO: call actual LLM (llama.cpp, ollama, etc.)
            let response = format!("[inferred] {}", prompt);

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}
