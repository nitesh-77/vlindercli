use crate::domain::{Agent, Model, Behavior, LoadError};

pub struct Runtime;

impl Runtime {
    pub fn new() -> Self {
        Runtime
    }

    pub fn spawn_agent(
        &self,
        name: &str,
        wasm_path: &str,
        model: Model,
        behavior: Behavior,
    ) -> Result<Agent, LoadError> {
        Agent::load(name, wasm_path, model, behavior)
    }
}
