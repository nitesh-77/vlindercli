//! ContainerPool — manages the lifecycle of Podman pods.
//!
//! Each agent runs as a pod containing two containers:
//! 1. The agent container (user-provided OCI image)
//! 2. The sidecar container (vlinder-sidecar, mediates queue ↔ agent)
//!
//! The pool creates pods, starts them, and tears them down on shutdown.
//! Dead pod detection is deferred — ensure_containers restarts missing pods
//! on the next tick.

use std::collections::HashMap;

use crate::config::Config;
use crate::domain::{Agent, ImageRef, PodId, Registry, RuntimeType};

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

/// A running pod: agent + sidecar containers sharing a network namespace.
struct Pod {
    pod_id: PodId,
}

/// Manages the lifecycle of Podman pods for container agents.
///
/// Maps agent names to running pods. Each pod contains an agent container
/// and a sidecar container. The sidecar handles dispatch; the pool handles
/// compute lifecycle.
pub(crate) struct ContainerPool {
    pods: HashMap<String, Pod>,
    config: Config,
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
            image_policy,
            podman,
        })
    }

    /// Start a pod with agent + sidecar containers.
    ///
    /// 1. Create a Podman pod named `vlinder-{name}`
    /// 2. Add the agent container (user image, no env vars)
    /// 3. Add the sidecar container (vlinder-sidecar image, env vars for config)
    /// 4. Start the pod (all containers start together)
    pub(crate) fn start(&mut self, name: &str, agent: &Agent) -> Result<(), String> {
        if self.pods.contains_key(name) {
            return Ok(());
        }

        let image_ref = ImageRef::parse(&agent.executable)
            .unwrap_or_else(|_| ImageRef::parse("unknown/unknown").unwrap());

        // Select what to pass to `podman run` based on policy (ADR 073)
        let run_target = match self.image_policy {
            ImagePolicy::Mutable => RunTarget::Ref(&image_ref),
            ImagePolicy::Pinned => agent.image_digest.as_ref()
                .map(RunTarget::Digest)
                .unwrap_or(RunTarget::Ref(&image_ref)),
        };

        // 1. Create pod
        let pod_name = format!("vlinder-{}", name);
        let pod_id = self.podman.pod_create(&pod_name)
            .map_err(|e| e.to_string())?;

        // 2. Add agent container (no env vars, no port mapping — shared network in pod)
        self.podman.container_in_pod(run_target, &pod_id, &[])
            .map_err(|e| e.to_string())?;

        // 3. Build sidecar env vars
        let sidecar_image_ref = ImageRef::parse(&self.config.runtime.sidecar_image)
            .unwrap_or_else(|_| ImageRef::parse("localhost/vlinder-sidecar:latest").unwrap());
        let sidecar_target = RunTarget::Ref(&sidecar_image_ref);

        let nats_url = format!(
            "nats://host.containers.internal:{}",
            extract_port(&self.config.queue.nats_url, 4222)
        );
        let registry_url = format!(
            "http://host.containers.internal:{}",
            extract_port(&self.config.distributed.registry_addr, 9090)
        );
        let state_url = format!(
            "http://host.containers.internal:{}",
            extract_port(&self.config.distributed.state_addr, 9092)
        );

        let image_digest_str = self.podman.image_digest(&image_ref)
            .map(|d| String::from(d))
            .unwrap_or_default();

        let env_vars: Vec<(&str, String)> = vec![
            ("VLINDER_AGENT", name.to_string()),
            ("VLINDER_NATS_URL", nats_url),
            ("VLINDER_REGISTRY_URL", registry_url),
            ("VLINDER_STATE_URL", state_url),
            ("VLINDER_CONTAINER_PORT", "8080".to_string()),
            ("VLINDER_IMAGE_REF", image_ref.as_str().to_string()),
            ("VLINDER_IMAGE_DIGEST", image_digest_str),
        ];
        let env_refs: Vec<(&str, &str)> = env_vars.iter()
            .map(|(k, v)| (*k, v.as_str()))
            .collect();

        // 4. Add sidecar container
        self.podman.container_in_pod(sidecar_target, &pod_id, &env_refs)
            .map_err(|e| e.to_string())?;

        // 5. Start the pod (all containers start together)
        self.podman.pod_start(&pod_id)
            .map_err(|e| e.to_string())?;

        tracing::info!(
            event = "pod.started",
            agent = %name,
            pod = %pod_id,
            image_ref = %image_ref,
            "Pod started (agent + sidecar)"
        );

        self.pods.insert(name.to_string(), Pod { pod_id });
        Ok(())
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
                    event = "pod.start_failed",
                    agent = %agent.name,
                    error = %e,
                    "Failed to start pod"
                );
            }
        }
    }

    /// Tick: ensure pods are running for all registered container agents.
    pub(crate) fn tick(&mut self, registry: &dyn Registry) -> bool {
        let before = self.pods.len();
        self.ensure_containers(registry);
        self.pods.len() != before
    }

    /// Shut down all managed pods.
    pub(crate) fn shutdown(&mut self) {
        for (name, pod) in self.pods.drain() {
            tracing::info!(event = "pod.stopped", agent = %name, pod = %pod.pod_id, "Stopping pod");
            self.podman.pod_stop_and_remove(&pod.pod_id, 5);
        }
    }
}

/// Extract port number from a URL string, with a default fallback.
fn extract_port(url: &str, default: u16) -> u16 {
    url.rsplit(':')
        .next()
        .and_then(|s| s.trim_end_matches('/').parse().ok())
        .unwrap_or(default)
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

    #[test]
    fn extract_port_nats_url() {
        assert_eq!(extract_port("nats://localhost:4222", 4222), 4222);
    }

    #[test]
    fn extract_port_http_url() {
        assert_eq!(extract_port("http://127.0.0.1:9090", 9090), 9090);
    }

    #[test]
    fn extract_port_custom_port() {
        assert_eq!(extract_port("nats://myhost:5555", 4222), 5555);
    }

    #[test]
    fn extract_port_no_port_returns_default() {
        assert_eq!(extract_port("nats://localhost", 4222), 4222);
    }

    #[test]
    fn extract_port_trailing_slash() {
        assert_eq!(extract_port("http://localhost:9090/", 9090), 9090);
    }
}
