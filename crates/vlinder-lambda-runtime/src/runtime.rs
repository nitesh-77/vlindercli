//! LambdaRuntime — stub implementation of the Runtime trait for AWS Lambda.
//!
//! Queries the registry for agents assigned to `RuntimeType::Lambda` and
//! logs their count. Actual Lambda invocation is deferred to a future spike.

use std::sync::Arc;

use vlinder_core::domain::{Registry, ResourceId, Runtime, RuntimeType};

use crate::config::LambdaRuntimeConfig;

/// Stub runtime for AWS Lambda agents.
///
/// Follows the same constructor pattern as `ContainerRuntime`:
/// receives config + registry, builds a ResourceId, implements `Runtime`.
pub struct LambdaRuntime {
    id: ResourceId,
    registry: Arc<dyn Registry>,
}

impl LambdaRuntime {
    /// Create a new Lambda runtime connected to the given registry.
    pub fn new(config: &LambdaRuntimeConfig, registry: Arc<dyn Registry>) -> Self {
        let registry_id = ResourceId::new(&config.registry_addr);
        let id = ResourceId::new(format!(
            "{}/runtimes/{}",
            registry_id.as_str(),
            RuntimeType::Lambda.as_str()
        ));

        Self { id, registry }
    }
}

impl Runtime for LambdaRuntime {
    fn id(&self) -> &ResourceId {
        &self.id
    }

    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Lambda
    }

    fn tick(&mut self) -> bool {
        let agents = self.registry.get_agents_by_runtime(RuntimeType::Lambda);
        if !agents.is_empty() {
            tracing::debug!(
                count = agents.len(),
                "Lambda agents discovered (stub — no invocation)"
            );
        }
        false
    }

    fn shutdown(&mut self) {
        // No resources to clean up (stub).
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vlinder_core::domain::InMemorySecretStore;

    fn test_config() -> LambdaRuntimeConfig {
        LambdaRuntimeConfig {
            registry_addr: "http://127.0.0.1:9090".to_string(),
        }
    }

    fn test_registry() -> Arc<dyn Registry> {
        use vlinder_core::domain::InMemoryRegistry;
        let secret_store = Arc::new(InMemorySecretStore::new());
        Arc::new(InMemoryRegistry::new(secret_store))
    }

    #[test]
    fn runtime_id_format() {
        let config = test_config();
        let registry = test_registry();
        let runtime = LambdaRuntime::new(&config, registry);

        assert_eq!(
            runtime.id().as_str(),
            "http://127.0.0.1:9090/runtimes/lambda"
        );
        assert_eq!(runtime.runtime_type(), RuntimeType::Lambda);
    }

    #[test]
    fn tick_returns_false_when_no_agents() {
        let config = test_config();
        let registry = test_registry();
        let mut runtime = LambdaRuntime::new(&config, registry);

        assert!(!runtime.tick());
    }
}
