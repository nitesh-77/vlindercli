//! ContainerRuntime — manages the lifecycle of Podman pods for container agents.
//!
//! Each agent runs as a pod containing two containers:
//! 1. The agent container (user-provided OCI image)
//! 2. The sidecar container (vlinder-sidecar, mediates queue ↔ agent)
//!
//! The runtime creates pods, starts them, and tears them down on shutdown.
//! Dead pod detection is deferred — ensure_containers restarts missing pods
//! on the next tick.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Config;
use crate::domain::{Agent, ImageRef, ObjectStorageType, PodId, Provider, Registry, ResourceId, Runtime, RuntimeType, VectorStorageType};

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

/// Orchestrates Podman pods for container agents.
///
/// Maps agent names to running pods. Each pod contains an agent container
/// and a sidecar container. The sidecar handles dispatch; the runtime handles
/// compute lifecycle.
pub struct ContainerRuntime {
    id: ResourceId,
    registry: Arc<dyn Registry>,
    pods: HashMap<String, Pod>,
    config: Config,
    image_policy: ImagePolicy,
    podman: Box<dyn Podman>,
}

impl ContainerRuntime {
    /// Create a new runtime, connecting to the registry and detecting Podman.
    ///
    /// Selects socket API or CLI based on the `podman_socket` config value (ADR 077).
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let registry = crate::registry_factory::from_config(config)?;

        let registry_id = ResourceId::new(&config.distributed.registry_addr);
        let id = ResourceId::new(format!(
            "{}/runtimes/{}",
            registry_id.as_str(),
            RuntimeType::Container.as_str()
        ));

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
            id,
            registry,
            pods: HashMap::new(),
            config: config.clone(),
            image_policy,
            podman,
        })
    }

    /// Access the registry (test-only, for integration test setup).
    #[cfg(any(test, feature = "test-support"))]
    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }

    /// Start a pod with agent + sidecar containers.
    ///
    /// 1. Create a Podman pod named `vlinder-{name}`
    /// 2. Add the agent container (user image, no env vars)
    /// 3. Add the sidecar container (vlinder-sidecar image, env vars for config)
    /// 4. Start the pod (all containers start together)
    fn start(&mut self, name: &str, agent: &Agent) -> Result<(), String> {
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

        // 1. Create pod (with host aliases for provider hostnames)
        let mut host_aliases = vec!["runtime.vlinder.local:127.0.0.1".to_string()];
        if agent.requirements.services.values().any(|svc| svc.provider == Provider::OpenRouter) {
            host_aliases.push(format!("{}:127.0.0.1", vlinder_infer_openrouter::HOSTNAME));
        }
        if agent.requirements.services.values().any(|svc| svc.provider == Provider::Ollama) {
            host_aliases.push(format!("{}:127.0.0.1", vlinder_ollama::HOSTNAME));
        }
        let needs_sqlite_vec = agent.vector_storage.as_ref()
            .and_then(|uri| VectorStorageType::from_scheme(uri.scheme()))
            .map(|t| t == VectorStorageType::SqliteVec)
            .unwrap_or(false);
        if needs_sqlite_vec {
            host_aliases.push(format!("{}:127.0.0.1", vlinder_sqlite_vec::HOSTNAME));
        }
        let needs_sqlite_kv = agent.object_storage.as_ref()
            .and_then(|uri| ObjectStorageType::from_scheme(uri.scheme()))
            .is_some();
        if needs_sqlite_kv {
            host_aliases.push(format!("{}:127.0.0.1", vlinder_sqlite_kv::HOSTNAME));
        }

        let pod_name = format!("vlinder-{}", name);
        let pod_id = self.podman.pod_create(&pod_name, &host_aliases)
            .map_err(|e| e.to_string())?;

        // From here on, if anything fails we must remove the orphaned pod.
        // Otherwise the next tick will try pod_create again and get "already exists".
        if let Err(e) = self.start_in_pod(name, &pod_id, run_target, &image_ref) {
            tracing::warn!(
                event = "pod.cleanup",
                agent = %name,
                pod = %pod_id,
                "Removing orphaned pod after start failure"
            );
            self.podman.pod_stop_and_remove(&pod_id, 0);
            return Err(e);
        }

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

    /// Populate and start a pod that has already been created.
    ///
    /// Adds the agent container, the sidecar container, and starts the pod.
    /// Called by `start()` — if this fails, `start()` cleans up the orphaned pod.
    fn start_in_pod(
        &self,
        name: &str,
        pod_id: &PodId,
        run_target: RunTarget<'_>,
        image_ref: &ImageRef,
    ) -> Result<(), String> {
        // 2. Add agent container (no env vars, no port mapping — shared network in pod)
        self.podman.container_in_pod(run_target, pod_id, &[])
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

        let image_digest_str = self.podman.image_digest(image_ref)
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
        self.podman.container_in_pod(sidecar_target, pod_id, &env_refs)
            .map_err(|e| e.to_string())?;

        // 5. Start the pod (all containers start together)
        self.podman.pod_start(pod_id)
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Start pods for agents that don't have one yet.
    fn ensure_containers(&mut self) {
        let agents = self.registry.get_agents_by_runtime(RuntimeType::Container);
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
        let before = self.pods.len();
        self.ensure_containers();
        self.pods.len() != before
    }

    fn shutdown(&mut self) {
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
    use crate::config::Config;

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
