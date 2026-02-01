//! Runtime trait - agent execution protocol.
//!
//! Defines how agents are registered and executed.

use super::Agent;

/// Executes agents in response to queue messages.
///
/// The runtime:
/// - Registers agents to serve
/// - Polls their input queues
/// - Executes agent code on message arrival
/// - Sends responses to reply queues
pub trait Runtime {
    /// Register an agent to be served by this runtime.
    fn register(&mut self, agent: Agent);

    /// Process agent work. Returns true if work was done.
    fn tick(&mut self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;

    /// Mock runtime for testing - records registrations, returns canned responses.
    struct MockRuntime {
        agents: HashMap<String, Agent>,
        work_available: bool,
    }

    impl MockRuntime {
        fn new() -> Self {
            Self {
                agents: HashMap::new(),
                work_available: false,
            }
        }

        fn set_work_available(&mut self, available: bool) {
            self.work_available = available;
        }

        fn has_agent(&self, name: &str) -> bool {
            self.agents.contains_key(name)
        }
    }

    impl Runtime for MockRuntime {
        fn register(&mut self, agent: Agent) {
            self.agents.insert(agent.name.clone(), agent);
        }

        fn tick(&mut self) -> bool {
            if self.work_available {
                self.work_available = false;
                true
            } else {
                false
            }
        }
    }

    fn load_test_agent(name: &str) -> Agent {
        let path = Path::new("tests/fixtures/agents").join(name);
        Agent::load(&path).unwrap()
    }

    #[test]
    fn mock_runtime_implements_trait() {
        let mut runtime: Box<dyn Runtime> = Box::new(MockRuntime::new());

        let agent = load_test_agent("reverse-agent");
        runtime.register(agent);

        // No work available
        assert!(!runtime.tick());
    }

    #[test]
    fn runtime_trait_is_object_safe() {
        // Can use Runtime as a trait object
        fn use_runtime(runtime: &mut dyn Runtime) {
            runtime.tick();
        }

        let mut mock = MockRuntime::new();
        mock.set_work_available(true);
        use_runtime(&mut mock);

        // Work was consumed
        assert!(!mock.work_available);
    }

    #[test]
    fn different_runtimes_same_interface() {
        // Mock runtime
        let mut mock = MockRuntime::new();
        let agent = load_test_agent("reverse-agent");
        mock.register(agent);
        assert!(mock.has_agent("reverse-agent"));

        // Both implement the same trait
        fn register_agent(runtime: &mut impl Runtime, agent: Agent) {
            runtime.register(agent);
        }

        let mut mock2 = MockRuntime::new();
        let agent2 = load_test_agent("echo-agent");
        register_agent(&mut mock2, agent2);
        assert!(mock2.has_agent("echo-agent"));
    }
}
