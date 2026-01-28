//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - Runtime: processes ExecutionPlan values (legacy model)
//! - WasmRuntime: queue-based WASM agent execution (new model)

mod wasm;

pub use wasm::WasmRuntime;

use crate::domain::{AgentExecution, ExecutionPlan};
use crate::executor::open_executor;
use crate::storage::{open_object_storage, open_vector_storage};

pub struct Runtime;

impl Runtime {
    pub fn new() -> Self {
        Runtime
    }

    /// Execute an ExecutionPlan to completion.
    pub fn execute_plan(&self, plan: ExecutionPlan) -> String {
        match plan {
            ExecutionPlan::Done(result) => result,
            ExecutionPlan::Continue(executions) => {
                // Execute all agents, collect results
                let results: Vec<String> = executions
                    .into_iter()
                    .map(|exec| self.execute_agent(exec))
                    .collect();
                results.join("\n")
            }
        }
    }

    /// Execute a single AgentExecution.
    ///
    /// Derives all capabilities (storage, executor) from the agent.
    pub fn execute_agent(&self, exec: AgentExecution) -> String {
        let agent = &exec.agent;

        let executor = match open_executor(agent) {
            Ok(e) => e,
            Err(e) => return format!("[error] {}", e),
        };

        let object_storage = match open_object_storage(agent) {
            Ok(s) => s,
            Err(e) => return format!("[error] failed to open object storage: {}", e),
        };

        let vector_storage = match open_vector_storage(agent) {
            Ok(s) => s,
            Err(e) => return format!("[error] failed to open vector storage: {}", e),
        };

        match executor.execute(agent, &exec.input, object_storage, vector_storage) {
            Ok(output) => output,
            Err(e) => format!("[error] {}", e),
        }
    }

    /// Execute an agent identified by URI with the provided input.
    ///
    /// Convenience method that loads the agent and calls execute_agent.
    pub fn execute(&self, uri: &str, input: &str) -> String {
        let agent = match crate::loader::load_agent(uri) {
            Ok(a) => a,
            Err(e) => return format!("[error] failed to load agent: {}", e),
        };

        self.execute_agent(AgentExecution {
            agent,
            input: input.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_plan_done_returns_result() {
        let runtime = Runtime::new();
        let plan = ExecutionPlan::Done("finished".to_string());

        let result = runtime.execute_plan(plan);

        assert_eq!(result, "finished");
    }

    #[test]
    fn execute_plan_continue_with_empty_returns_empty() {
        let runtime = Runtime::new();
        let plan = ExecutionPlan::Continue(vec![]);

        let result = runtime.execute_plan(plan);

        assert_eq!(result, "");
    }
}
