//! Pod — the deployable unit managed by the pool.
//!
//! A Pod = Container + Sidecar. Container holds OCI lifecycle state,
//! Sidecar mediates between queue and container.

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{Agent, ContainerId, ImageDigest, ImageRef, MessageQueue, ObjectStorageType, QueueBridge, Registry, InvokeMessage, SequenceCounter, VectorStorageType};

/// The OCI container half of a Pod.
pub(super) struct Container {
    pub(super) container_id: ContainerId,
    pub(super) host_port: u16,
    /// The OCI image reference (always `agent.executable` — identifies *which* image).
    pub(super) image_ref: ImageRef,
    /// Content-addressed digest from `podman image inspect` at container start.
    /// None if the inspect failed.
    pub(super) image_digest: Option<ImageDigest>,
}

/// The sidecar half of a Pod — mediates between queue and container.
pub(super) struct Sidecar {
    config: Config,
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    kv_backend: Option<ObjectStorageType>,
    vec_backend: Option<VectorStorageType>,
}

impl Sidecar {
    pub(super) fn new(config: &Config, agent: &Agent) -> Result<Self, Box<dyn std::error::Error>> {
        let queue = crate::queue_factory::recording_from_config(config)?;
        let registry = crate::registry_factory::from_config(config)?;
        let kv_backend = agent.object_storage.as_ref()
            .and_then(|uri| ObjectStorageType::from_scheme(uri.scheme()));
        let vec_backend = agent.vector_storage.as_ref()
            .and_then(|uri| VectorStorageType::from_scheme(uri.scheme()));
        Ok(Self { config: config.clone(), queue, registry, kv_backend, vec_backend })
    }

    pub(super) fn build_bridge(&self, invoke: &InvokeMessage) -> Arc<QueueBridge> {
        // Bootstrap state to root ("") if agent uses KV but no prior state exists (ADR 055).
        let initial_state = invoke.state.clone()
            .or_else(|| self.kv_backend.as_ref().map(|_| String::new()));
        Arc::new(QueueBridge {
            queue: Arc::clone(&self.queue),
            registry: Arc::clone(&self.registry),
            current_state: std::sync::RwLock::new(initial_state),
            invoke: std::sync::RwLock::new(invoke.clone()),
            kv_backend: self.kv_backend,
            vec_backend: self.vec_backend,
            sequence: SequenceCounter::new(),
            pending_replies: std::sync::RwLock::new(std::collections::HashMap::new()),
        })
    }
}

/// A Pod = Container + Sidecar. The deployable unit managed by the pool.
pub(super) struct Pod {
    pub(super) container: Container,
    pub(super) sidecar: Sidecar,
    pub(super) bridge: Arc<QueueBridge>,
}
