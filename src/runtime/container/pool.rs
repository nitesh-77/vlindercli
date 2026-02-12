//! ContainerPool — manages the lifecycle of long-running OCI containers.
//!
//! Owns the container map, Podman engine, and image resolution policy.
//! The tick-loop scheduler in `ContainerRuntime` delegates all container
//! lifecycle operations here: lazy start, eviction, shutdown, diagnostics.

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::{Agent, QueueBridge, ContainerDiagnostics, ContainerRuntimeInfo, InvokeMessage};

use super::podman::{Podman, PodmanCli};

/// A long-running container managed by the pool.
pub(super) struct ManagedContainer {
    container_id: String,
    host_port: u16,
    pub(super) bridge: Arc<QueueBridge>,
    /// What was passed to `podman run` (tag in mutable mode, digest in pinned mode).
    image_ref: String,
    /// Content-addressed digest from `podman image inspect` at container start.
    /// None if the inspect failed.
    image_digest: Option<String>,
}

/// Image resolution policy for container agents (ADR 073).
///
/// Controls which OCI reference is passed to `podman run`:
/// - `Mutable`: Uses the tag from `agent.executable` — rebuilt images picked up automatically.
/// - `Pinned`: Uses the content-addressed digest from `agent.image_digest` — deterministic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImagePolicy {
    Mutable,
    Pinned,
}

impl ImagePolicy {
    pub fn from_config(s: &str) -> Self {
        match s {
            "pinned" => Self::Pinned,
            _ => Self::Mutable,
        }
    }
}

/// Manages the lifecycle of long-running OCI containers.
///
/// Maps agent names to running containers, lazily starts them on first
/// invocation, and tears them down on eviction or shutdown.
pub(crate) struct ContainerPool {
    containers: HashMap<String, ManagedContainer>,
    engine_version: Option<semver::Version>,
    image_policy: ImagePolicy,
    podman: Box<dyn Podman>,
}

impl ContainerPool {
    /// Create a new pool, detecting the Podman engine version.
    pub(crate) fn new(image_policy: ImagePolicy) -> Self {
        let podman = Box::new(PodmanCli);
        let engine_version = podman.engine_version();
        if let Some(ref v) = engine_version {
            tracing::info!(event = "podman.detected", version = %v, "Podman engine detected");
        } else {
            tracing::warn!(event = "podman.not_found", "Podman not detected — container runtime degraded");
        }
        tracing::info!(event = "runtime.image_policy", policy = ?image_policy, "Container image policy");
        Self {
            containers: HashMap::new(),
            engine_version,
            image_policy,
            podman,
        }
    }

    /// If a container is already running for `name`, update its bridge with the
    /// new invoke context and return the host port and bridge.
    pub(crate) fn get_port(&self, name: &str, invoke: &InvokeMessage) -> Option<(u16, Arc<QueueBridge>)> {
        self.containers.get(name).map(|mc| {
            mc.bridge.update_invoke(invoke.clone());
            (mc.host_port, Arc::clone(&mc.bridge))
        })
    }

    /// Start a new container for `agent`, wiring it to `bridge`.
    ///
    /// Selects the image reference based on the image policy, runs the container
    /// via Podman, discovers the mapped port, and waits for readiness.
    pub(crate) fn start(
        &mut self,
        name: &str,
        agent: &Agent,
        bridge: Arc<QueueBridge>,
    ) -> Result<(u16, Arc<QueueBridge>), String> {
        // Select image reference based on policy (ADR 073)
        let image = match self.image_policy {
            ImagePolicy::Mutable => agent.executable.clone(),
            ImagePolicy::Pinned => agent.image_digest.clone()
                .unwrap_or_else(|| agent.executable.clone()),
        };

        // Build volume mount flags from agent manifest (ADR 057)
        let mount_flags: Vec<String> = agent.mounts.iter().map(|m| {
            let mode = if m.readonly { "ro" } else { "rw" };
            format!("{}:{}:{}", m.host_path, m.guest_path.display(), mode)
        }).collect();

        let container_id = self.podman.run(&image, &mount_flags)?;

        // Capture image metadata for diagnostics (ADR 073)
        let image_digest = self.podman.image_digest(&image);
        let image_ref = image;

        // Discover the mapped host port
        let host_port = self.podman.port(&container_id)?;

        // Wait for container to be ready
        self.podman.wait_for_ready(host_port)?;

        tracing::info!(
            event = "container.started",
            agent = %name,
            container = %container_id,
            port = host_port,
            image_ref = %image_ref,
            image_digest = image_digest.as_deref().unwrap_or("unknown"),
            "Container started"
        );

        let bridge_clone = Arc::clone(&bridge);
        self.containers.insert(name.to_string(), ManagedContainer {
            container_id,
            host_port,
            bridge,
            image_ref,
            image_digest,
        });

        Ok((host_port, bridge_clone))
    }

    /// Evict a stale container — stop, remove, and drop the bridge (ADR 073).
    ///
    /// Called when dispatch detects a transport error (container dead).
    pub(crate) fn evict(&mut self, agent_name: &str) {
        if let Some(mc) = self.containers.remove(agent_name) {
            tracing::warn!(
                event = "container.evicted",
                agent = %agent_name,
                container = %mc.container_id,
                "Evicting stale container"
            );
            self.podman.stop_and_remove(&mc.container_id, 2);
        }
    }

    /// Shut down all managed containers.
    pub(crate) fn shutdown(&mut self) {
        for (name, mc) in self.containers.drain() {
            tracing::info!(event = "container.stopped", agent = %name, container = %mc.container_id, "Stopping container");
            self.podman.stop_and_remove(&mc.container_id, 5);
        }
    }

    /// Build ContainerDiagnostics from cached metadata (ADR 073).
    pub(crate) fn diagnostics(&self, agent_name: &str, duration_ms: u64) -> ContainerDiagnostics {
        match self.containers.get(agent_name) {
            Some(mc) => ContainerDiagnostics {
                stderr: Vec::new(),
                runtime: ContainerRuntimeInfo {
                    engine_version: self.engine_version.as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    image_ref: mc.image_ref.clone(),
                    image_digest: mc.image_digest.clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    container_id: mc.container_id.clone(),
                },
                duration_ms,
            },
            None => ContainerDiagnostics::placeholder(duration_ms),
        }
    }

    /// Retrieve the final state hash from the bridge for a named container.
    pub(crate) fn final_state(&self, name: &str) -> Option<String> {
        self.containers.get(name).and_then(|mc| mc.bridge.final_state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_policy_from_config_pinned() {
        assert_eq!(ImagePolicy::from_config("pinned"), ImagePolicy::Pinned);
    }

    #[test]
    fn image_policy_from_config_mutable() {
        assert_eq!(ImagePolicy::from_config("mutable"), ImagePolicy::Mutable);
    }

    #[test]
    fn image_policy_from_config_default_is_mutable() {
        assert_eq!(ImagePolicy::from_config(""), ImagePolicy::Mutable);
        assert_eq!(ImagePolicy::from_config("unknown"), ImagePolicy::Mutable);
    }
}
