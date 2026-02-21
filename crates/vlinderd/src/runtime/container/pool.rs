//! ContainerPool — manages the lifecycle of long-running OCI containers.
//!
//! Owns the pod map, Podman engine, and image resolution policy.
//! Each pod is a container + sidecar thread. The pool starts containers,
//! spawns sidecar threads, reaps dead threads, and shuts everything down.

use std::collections::HashMap;
use std::thread::{self, JoinHandle};

use crate::config::Config;
use crate::domain::{Agent, ContainerId, ImageRef, Registry, RuntimeType};

use super::pod::{Container, Sidecar};
use super::podman::{Podman, RunTarget, resolve_socket};
use super::podman_api::PodmanApiClient;
use super::podman_cli::PodmanCliClient;

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

/// A running pod: container ID (for stop/remove) + sidecar thread handle.
struct Pod {
    container_id: ContainerId,
    handle: JoinHandle<()>,
}

/// Manages the lifecycle of long-running OCI containers.
///
/// Maps agent names to running pods, starts containers with sidecar threads,
/// reaps dead threads, and tears everything down on shutdown.
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

    /// Start a container and sidecar thread for `agent`.
    ///
    /// Runs the container via Podman, spawns a sidecar thread running
    /// `Sidecar::run()`, and stores the pod in the map.
    pub(crate) fn start(&mut self, name: &str, agent: &Agent) -> Result<(), String> {
        if self.pods.contains_key(name) {
            return Ok(());
        }

        let sidecar = Sidecar::new(&self.config, agent)
            .map_err(|e| e.to_string())?;

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

        let container = Container {
            container_id: container_id.clone(),
            image_ref,
            image_digest,
        };

        let engine_version = self.engine_version.clone();
        let agent_name = name.to_string();
        let handle = thread::spawn(move || {
            sidecar.run(agent_name, host_port, container, engine_version);
        });

        self.pods.insert(name.to_string(), Pod { container_id, handle });
        Ok(())
    }

    /// Check which sidecar threads have exited (container died), clean up those pods.
    fn reap_dead(&mut self) {
        let finished: Vec<String> = self.pods.iter()
            .filter(|(_, pod)| pod.handle.is_finished())
            .map(|(name, _)| name.clone())
            .collect();

        for name in finished {
            let pod = self.pods.remove(&name).unwrap();
            tracing::warn!(
                event = "sidecar.reaped",
                agent = %name,
                container = %pod.container_id,
                "Sidecar thread exited — stopping container"
            );
            self.podman.stop_and_remove(&pod.container_id, 2);
        }
    }

    /// Start pods for agents that don't have one yet.
    fn ensure_containers(&mut self, registry: &dyn Registry) {
        let agents = registry.get_agents_by_runtime(RuntimeType::Container);
        for agent in &agents {
            if self.pods.contains_key(&agent.name) {
                continue;
            }
            if let Err(e) = self.start(&agent.name, agent) {
                tracing::error!(
                    event = "container.start_failed",
                    agent = %agent.name,
                    error = %e,
                    "Failed to start container"
                );
            }
        }
    }

    /// Tick: ensure containers are running, reap dead sidecar threads.
    pub(crate) fn tick(&mut self, registry: &dyn Registry) -> bool {
        let before = self.pods.len();
        self.reap_dead();
        self.ensure_containers(registry);
        // Did work if pods changed (reaped or started)
        self.pods.len() != before
    }

    /// Shut down all managed containers.
    pub(crate) fn shutdown(&mut self) {
        for (name, pod) in self.pods.drain() {
            tracing::info!(event = "container.stopped", agent = %name, container = %pod.container_id, "Stopping container");
            self.podman.stop_and_remove(&pod.container_id, 5);
        }
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
