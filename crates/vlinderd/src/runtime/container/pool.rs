//! ContainerPool — manages the lifecycle of long-running OCI containers.
//!
//! Owns the container map, Podman engine, and image resolution policy.
//! The tick-loop scheduler in `ContainerRuntime` delegates all container
//! lifecycle operations here: lazy start, eviction, shutdown, diagnostics.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Config;
use crate::domain::{Agent, ContainerId, ImageDigest, ImageRef, MessageQueue, ObjectStorageType, QueueBridge, Registry, ContainerDiagnostics, ContainerRuntimeInfo, InvokeMessage, SequenceCounter, VectorStorageType};

use super::podman::{Podman, RunTarget, resolve_socket};
use super::podman_api::PodmanApiClient;
use super::podman_cli::PodmanCliClient;

/// The OCI container half of a Pod.
struct Container {
    container_id: ContainerId,
    host_port: u16,
    /// The OCI image reference (always `agent.executable` — identifies *which* image).
    image_ref: ImageRef,
    /// Content-addressed digest from `podman image inspect` at container start.
    /// None if the inspect failed.
    image_digest: Option<ImageDigest>,
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
    fn new(config: &Config, agent: &Agent) -> Result<Self, Box<dyn std::error::Error>> {
        let queue = crate::queue_factory::recording_from_config(config)?;
        let registry = crate::registry_factory::from_config(config)?;
        let kv_backend = agent.object_storage.as_ref()
            .and_then(|uri| ObjectStorageType::from_scheme(uri.scheme()));
        let vec_backend = agent.vector_storage.as_ref()
            .and_then(|uri| VectorStorageType::from_scheme(uri.scheme()));
        Ok(Self { config: config.clone(), queue, registry, kv_backend, vec_backend })
    }

    fn build_bridge(&self, invoke: &InvokeMessage) -> Arc<QueueBridge> {
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
struct Pod {
    container: Container,
    sidecar: Sidecar,
    bridge: Arc<QueueBridge>,
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
    pods: HashMap<String, Pod>,
    config: Config,
    engine_version: Option<semver::Version>,
    image_policy: ImagePolicy,
    podman: Box<dyn Podman>,
}

impl ContainerPool {
    /// Create a new pool, detecting the Podman engine version.
    ///
    /// Selects socket API or CLI based on the `podman_socket` config value (ADR 077).
    pub(crate) fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let image_policy = ImagePolicy::from_config(&config.runtime.image_policy);
        let podman: Box<dyn Podman> = match resolve_socket(&config.runtime.podman_socket) {
            Some(path) => {
                tracing::info!(event = "podman.socket", path = %path.display(), "Using Podman socket API");
                Box::new(PodmanApiClient::new(&path))
            }
            None => {
                tracing::info!(event = "podman.cli", "Using Podman CLI");
                Box::new(PodmanCliClient)
            }
        };

        let engine_version = podman.engine_version();
        if let Some(ref v) = engine_version {
            tracing::info!(event = "podman.detected", version = %v, "Podman engine detected");
        } else {
            tracing::warn!(event = "podman.not_found", "Podman not detected — container runtime degraded");
        }
        tracing::info!(event = "runtime.image_policy", policy = ?image_policy, "Container image policy");
        Ok(Self {
            pods: HashMap::new(),
            config: config.clone(),
            engine_version,
            image_policy,
            podman,
        })
    }

    /// If a container is already running for `name`, update its bridge with the
    /// new invoke context and return the host port and bridge.
    pub(crate) fn get_port(&self, name: &str, invoke: &InvokeMessage) -> Option<(u16, Arc<QueueBridge>)> {
        self.pods.get(name).map(|pod| {
            pod.bridge.update_invoke(invoke.clone());
            (pod.container.host_port, Arc::clone(&pod.bridge))
        })
    }

    /// Start a new container for `agent`.
    ///
    /// Builds the QueueBridge, selects the image reference based on the image
    /// policy, runs the container via Podman, discovers the mapped port, and
    /// waits for readiness.
    pub(crate) fn start(
        &mut self,
        name: &str,
        agent: &Agent,
        invoke: &InvokeMessage,
    ) -> Result<(u16, Arc<QueueBridge>), String> {
        let sidecar = Sidecar::new(&self.config, agent)
            .map_err(|e| e.to_string())?;
        let bridge = sidecar.build_bridge(invoke);
        // image_ref always records *which* image, not *which bytes*
        let image_ref = ImageRef::parse(&agent.executable)
            .unwrap_or_else(|_| ImageRef::parse("unknown/unknown").unwrap());

        // Select what to pass to `podman run` based on policy (ADR 073)
        let run_target = match self.image_policy {
            ImagePolicy::Mutable => RunTarget::Ref(&image_ref),
            ImagePolicy::Pinned => agent.image_digest.as_ref()
                .map(RunTarget::Digest)
                .unwrap_or(RunTarget::Ref(&image_ref)),
        };

        let container_id = self.podman.run(run_target)
            .map_err(|e| e.to_string())?;

        // Capture image metadata for diagnostics (ADR 073)
        let image_digest = self.podman.image_digest(&image_ref);

        // Discover the mapped host port
        let host_port = self.podman.port(&container_id)
            .map_err(|e| e.to_string())?;

        // Wait for container to be ready
        self.podman.wait_for_ready(host_port)
            .map_err(|e| e.to_string())?;

        tracing::info!(
            event = "container.started",
            agent = %name,
            container = %container_id,
            port = host_port,
            image_ref = %image_ref,
            image_digest = image_digest.as_ref().map(|d| d.as_str()).unwrap_or("unknown"),
            "Container started"
        );

        let bridge_clone = Arc::clone(&bridge);
        self.pods.insert(name.to_string(), Pod {
            container: Container {
                container_id,
                host_port,
                image_ref,
                image_digest,
            },
            sidecar,
            bridge,
        });

        Ok((host_port, bridge_clone))
    }

    /// Evict a stale container — stop, remove, and drop the bridge (ADR 073).
    ///
    /// Called when dispatch detects a transport error (container dead).
    pub(crate) fn evict(&mut self, agent_name: &str) {
        if let Some(pod) = self.pods.remove(agent_name) {
            tracing::warn!(
                event = "container.evicted",
                agent = %agent_name,
                container = %pod.container.container_id,
                "Evicting stale container"
            );
            self.podman.stop_and_remove(&pod.container.container_id, 2);
        }
    }

    /// Shut down all managed containers.
    pub(crate) fn shutdown(&mut self) {
        for (name, pod) in self.pods.drain() {
            tracing::info!(event = "container.stopped", agent = %name, container = %pod.container.container_id, "Stopping container");
            self.podman.stop_and_remove(&pod.container.container_id, 5);
        }
    }

    /// Build ContainerDiagnostics from cached metadata (ADR 073).
    pub(crate) fn diagnostics(&self, agent_name: &str, duration_ms: u64) -> ContainerDiagnostics {
        match self.pods.get(agent_name) {
            Some(pod) => ContainerDiagnostics {
                stderr: Vec::new(),
                runtime: ContainerRuntimeInfo {
                    engine_version: self.engine_version.as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    image_ref: Some(pod.container.image_ref.clone()),
                    image_digest: pod.container.image_digest.clone(),
                    container_id: pod.container.container_id.clone(),
                },
                duration_ms,
            },
            None => ContainerDiagnostics::placeholder(duration_ms),
        }
    }

    /// Retrieve the final state hash from the bridge for a named container.
    pub(crate) fn final_state(&self, name: &str) -> Option<String> {
        self.pods.get(name).and_then(|pod| pod.bridge.final_state())
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
