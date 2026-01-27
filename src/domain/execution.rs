//! Execution domain types.
//!
//! These types represent what the runtime processes.

use super::agent::Agent;
use super::embedding::Embedding;
use super::executor::Executor;
use super::inference::Inference;
use super::storage::Storage;

/// A plan describing what the runtime should execute.
#[derive(Clone, Debug)]
pub enum ExecutionPlan {
    /// Execution is complete with this final result.
    Done(String),
    /// Continue with these agent executions.
    Continue(Vec<AgentExecution>),
}

/// Everything needed to execute a single agent.
#[derive(Clone, Debug)]
pub struct AgentExecution {
    pub agent: Agent,
    pub executor: Executor,
    pub storage: Option<Storage>,
    pub inference: Option<Inference>,
    pub embedding: Option<Embedding>,
    pub input: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ExecutorBackend, Requirements};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn test_agent() -> Agent {
        Agent {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            source: None,
            code: "file://test.wasm".to_string(),
            agent_dir: PathBuf::from("/tmp"),
            prompts: None,
            requirements: Requirements {
                models: HashMap::new(),
                services: vec![],
            },
            mounts: vec![],
        }
    }

    fn test_executor() -> Executor {
        Executor {
            backend: ExecutorBackend::default(),
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
            executor: test_executor(),
            storage: None,
            inference: None,
            embedding: None,
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
    fn agent_execution_with_no_capabilities() {
        let exec = AgentExecution {
            agent: test_agent(),
            executor: test_executor(),
            storage: None,
            inference: None,
            embedding: None,
            input: "test input".to_string(),
        };

        assert_eq!(exec.agent.name, "test-agent");
        assert!(exec.storage.is_none());
        assert!(exec.inference.is_none());
        assert!(exec.embedding.is_none());
    }
}
