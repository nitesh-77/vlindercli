//! Agent runtime - orchestrates agent execution.
//!
//! The runtime processes ExecutionPlan values:
//! - `Done(result)`: Return the result
//! - `Continue(executions)`: Execute all agents and continue

use crate::domain::{AgentExecution, ExecutionPlan};
use crate::executor::open_executor;
use crate::executor::dispatch::open_executor as dispatch_open_executor;
use crate::loader;
use crate::storage::dispatch::{open_object_storage as dispatch_object, open_vector_storage as dispatch_vector};
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

    /// Execute a single AgentExecution using domain types.
    pub fn execute_agent(&self, exec: AgentExecution) -> String {
        // Open storage from domain type if present
        let (object_storage, vector_storage) = if let Some(ref storage) = exec.storage {
            let obj = match dispatch_object(storage) {
                Ok(s) => s,
                Err(e) => return format!("[error] failed to open object storage: {}", e),
            };
            let vec = match dispatch_vector(storage) {
                Ok(s) => s,
                Err(e) => return format!("[error] failed to open vector storage: {}", e),
            };
            (obj, vec)
        } else {
            // Fallback to legacy storage opening
            let obj = match open_object_storage(&exec.agent) {
                Ok(s) => s,
                Err(e) => return format!("[error] failed to open object storage: {}", e),
            };
            let vec = match open_vector_storage(&exec.agent) {
                Ok(s) => s,
                Err(e) => return format!("[error] failed to open vector storage: {}", e),
            };
            (obj, vec)
        };

        // Open executor from domain type
        let executor = match dispatch_open_executor(&exec.executor.backend.kind) {
            Ok(e) => e,
            Err(e) => return format!("[error] failed to open executor: {}", e),
        };

        match executor.execute(&exec.agent, &exec.input, object_storage, vector_storage) {
            Ok(output) => output,
            Err(e) => format!("[error] {}", e),
        }
    }

    /// Execute an agent identified by URI with the provided input.
    ///
    /// Legacy entry point - builds domain types internally.
    pub fn execute(&self, uri: &str, input: &str) -> String {
        let agent = match loader::load_agent(uri) {
            Ok(a) => a,
            Err(e) => return format!("[error] failed to load agent: {}", e),
        };

        let executor = match open_executor(&agent) {
            Ok(e) => e,
            Err(e) => return format!("[error] {}", e),
        };

        let object_storage = match open_object_storage(&agent) {
            Ok(s) => s,
            Err(e) => return format!("[error] failed to open object storage: {}", e),
        };

        let vector_storage = match open_vector_storage(&agent) {
            Ok(s) => s,
            Err(e) => return format!("[error] failed to open vector storage: {}", e),
        };

        match executor.execute(&agent, input, object_storage, vector_storage) {
            Ok(output) => output,
            Err(e) => format!("[error] {}", e),
        }
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
