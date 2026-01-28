//! Execution domain types.
//!
//! These types represent what the runtime processes.

use super::agent::Agent;

/// A plan describing what the runtime should execute.
#[derive(Clone, Debug)]
pub enum ExecutionPlan {
    /// Execution is complete with this final result.
    Done(String),
    /// Continue with these agent executions.
    Continue(Vec<AgentExecution>),
}

/// A request to execute an agent with input.
///
/// The runtime derives all capabilities (storage, executor, inference, embedding)
/// from the agent's configuration.
#[derive(Clone, Debug)]
pub struct AgentExecution {
    pub agent: Agent,
    pub input: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AbsoluteUri, Requirements};
    use std::collections::HashMap;

    fn test_agent() -> Agent {
        Agent {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            source: None,
            code: AbsoluteUri::from_absolute("file:///tmp/test.wasm").unwrap(),
            prompts: None,
            requirements: Requirements {
                models: HashMap::new(),
                services: vec![],
            },
            mounts: vec![],
        }
    }

    #[test]
    fn execution_plan_done() {
        let plan = ExecutionPlan::Done("result".to_string());

        match plan {
            ExecutionPlan::Done(result) => assert_eq!(result, "result"),
            ExecutionPlan::Continue(_) => panic!("expected Done"),
        }
    }

    #[test]
    fn execution_plan_continue() {
        let exec = AgentExecution {
            agent: test_agent(),
            input: "hello".to_string(),
        };

        let plan = ExecutionPlan::Continue(vec![exec]);

        match plan {
            ExecutionPlan::Done(_) => panic!("expected Continue"),
            ExecutionPlan::Continue(execs) => {
                assert_eq!(execs.len(), 1);
                assert_eq!(execs[0].input, "hello");
            }
        }
    }

    #[test]
    fn agent_execution_holds_agent_and_input() {
        let exec = AgentExecution {
            agent: test_agent(),
            input: "test input".to_string(),
        };

        assert_eq!(exec.agent.name, "test-agent");
        assert_eq!(exec.input, "test input");
    }
}
