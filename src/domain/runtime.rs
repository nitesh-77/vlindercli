//! Runtime trait - agent execution protocol.
//!
//! Defines how agents are registered and executed.

use super::ResourceId;

// ============================================================================
// Runtime Type (compile-time supported runtimes)
// ============================================================================

/// Supported runtime types.
///
/// This is a compile-time enum - adding a new runtime type requires code changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeType {
    /// OCI container runtime (Podman)
    Container,
}

impl RuntimeType {
    /// String representation for URI construction.
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeType::Container => "container",
        }
    }

    /// Parse from manifest string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "container" => Some(RuntimeType::Container),
            _ => None,
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
/// - Discovers agents from Registry
/// - Polls their input queues
/// - Executes agent code on message arrival
/// - Sends responses to reply queues
pub trait Runtime {
    /// Unique identifier for this runtime instance.
    /// Format: `<registry_id>/runtimes/<runtime_type>`
    fn id(&self) -> &ResourceId;

    /// The type of runtime (Container, etc.)
    fn runtime_type(&self) -> RuntimeType;

    /// Process agent work. Returns true if work was done.
    fn tick(&mut self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock runtime for testing.
    struct MockRuntime {
        id: ResourceId,
        work_available: bool,
    }

    impl MockRuntime {
        fn new() -> Self {
            Self {
                id: ResourceId::new("http://test/runtimes/mock"),
                work_available: false,
            }
        }

        fn set_work_available(&mut self, available: bool) {
            self.work_available = available;
        }
    }

    impl Runtime for MockRuntime {
        fn id(&self) -> &ResourceId {
            &self.id
        }

        fn runtime_type(&self) -> RuntimeType {
            RuntimeType::Container
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

    #[test]
    fn mock_runtime_implements_trait() {
        let mut runtime: Box<dyn Runtime> = Box::new(MockRuntime::new());

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
}
