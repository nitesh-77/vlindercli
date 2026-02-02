//! Runtime trait - agent execution protocol.
//!
//! Defines how agents are registered and executed.

use super::{Agent, ResourceId};

// ============================================================================
// Runtime Type (compile-time supported runtimes)
// ============================================================================

/// Supported runtime types.
///
/// This is a compile-time enum - adding a new runtime type requires code changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeType {
    /// WebAssembly runtime (Extism/WASI)
    Wasm,
    // Future: Lambda, Container, etc.
}

impl RuntimeType {
    /// String representation for URI construction.
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeType::Wasm => "wasm",
        }
    }
}

// ============================================================================
// Runtime Trait
// ============================================================================

/// Executes agents in response to queue messages.
///
/// The runtime:
/// - Has a unique id (`<registry>/runtimes/<type>`)
/// - Registers agents to serve
/// - Polls their input queues
/// - Executes agent code on message arrival
/// - Sends responses to reply queues
pub trait Runtime {
    /// Unique identifier for this runtime instance.
    /// Format: `<registry_id>/runtimes/<runtime_type>`
    fn id(&self) -> &ResourceId;

    /// The type of runtime (Wasm, Lambda, etc.)
    fn runtime_type(&self) -> RuntimeType;

    /// Register an agent to be served by this runtime.
    fn register(&mut self, agent: Agent);

    /// Process agent work. Returns true if work was done.
    fn tick(&mut self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Requirements;
    use std::collections::HashMap;

    /// Mock runtime for testing - records registrations, returns canned responses.
    struct MockRuntime {
        id: ResourceId,
        agents: HashMap<String, Agent>,
        work_available: bool,
    }

    impl MockRuntime {
        fn new() -> Self {
            Self {
                id: ResourceId::new("http://test/runtimes/mock"),
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
        fn id(&self) -> &ResourceId {
            &self.id
        }

        fn runtime_type(&self) -> RuntimeType {
            RuntimeType::Wasm // Mock as Wasm for testing
        }

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

    fn mock_agent(name: &str) -> Agent {
        Agent {
            name: name.to_string(),
            description: format!("Mock {} for testing", name),
            source: None,
            requirements: Requirements {
                models: HashMap::new(),
                services: vec![],
            },
            prompts: None,
            mounts: vec![],
            id: ResourceId::new(format!("file:///mock/{}.wasm", name)),
            object_storage: None,
            vector_storage: None,
        }
    }

    #[test]
    fn mock_runtime_implements_trait() {
        let mut runtime: Box<dyn Runtime> = Box::new(MockRuntime::new());

        let agent = mock_agent("test-agent");
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
        let agent = mock_agent("agent-a");
        mock.register(agent);
        assert!(mock.has_agent("agent-a"));

        // Both implement the same trait
        fn register_agent(runtime: &mut impl Runtime, agent: Agent) {
            runtime.register(agent);
        }

        let mut mock2 = MockRuntime::new();
        let agent2 = mock_agent("agent-b");
        register_agent(&mut mock2, agent2);
        assert!(mock2.has_agent("agent-b"));
    }
}
