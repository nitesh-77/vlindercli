//! ContainerRuntime — thin orchestrator for OCI container agents.
//!
//! Owns a registry and a pool. On each tick, tells the pool to ensure
//! containers are running and reap dead sidecar threads. All dispatch
//! logic lives in Sidecar; all container lifecycle logic lives in Pool.

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{Registry, ResourceId, Runtime, RuntimeType};

use super::pool::ContainerPool;

pub struct ContainerRuntime {
    id: ResourceId,
    registry: Arc<dyn Registry>,
    pool: ContainerPool,
}

impl ContainerRuntime {
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let registry = crate::registry_factory::from_config(config)?;

        let registry_id = ResourceId::new(&config.distributed.registry_addr);

        let id = ResourceId::new(format!(
            "{}/runtimes/{}",
            registry_id.as_str(),
            RuntimeType::Container.as_str()
        ));
        Ok(Self {
            id,
            registry,
            pool: ContainerPool::new(config)?,
        })
    }

    /// Access the registry (test-only, for integration test setup).
    #[cfg(any(test, feature = "test-support"))]
    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }
}

impl Drop for ContainerRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Runtime for ContainerRuntime {
    fn id(&self) -> &ResourceId {
        &self.id
    }

    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Container
    }

    fn tick(&mut self) -> bool {
        self.pool.tick(self.registry.as_ref())
    }

    fn shutdown(&mut self) {
        self.pool.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn runtime_id_format() {
        let runtime = ContainerRuntime::new(&Config::for_test()).unwrap();

        assert_eq!(
            runtime.id().as_str(),
            "http://127.0.0.1:9090/runtimes/container"
        );
        assert_eq!(runtime.runtime_type(), RuntimeType::Container);
    }

    #[test]
    fn tick_returns_false_when_no_agents() {
        let mut runtime = ContainerRuntime::new(&Config::for_test()).unwrap();

        assert!(!runtime.tick());
    }
}
