//! PodmanApiClient — Podman trait implementation using the libpod REST API.
//!
//! Primary implementation. Talks to Podman's REST API over a Unix socket.
//! Uses ureq with a Unix transport adapter for HTTP-over-socket.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::{ContainerId, ImageDigest, ImageRef, PodId};

use super::podman::{Podman, PodmanError, RunTarget};
use super::unix_transport::unix_agent;

const API_BASE: &str = "http://localhost/v5.0.0/libpod";

/// Podman REST API client — primary implementation.
///
/// Talks to the libpod REST API over a Unix socket. The socket path is
/// resolved by `resolve_socket()` in `podman.rs` (ADR 077).
pub(crate) struct PodmanApiClient {
    agent: ureq::Agent,
}

impl PodmanApiClient {
    pub(crate) fn new(socket_path: &Path) -> Self {
        Self {
            agent: unix_agent(socket_path),
        }
    }
}

// ── Podman trait implementation ──────────────────────────────────────

impl Podman for PodmanApiClient {
    fn engine_version(&self) -> Option<semver::Version> {
        let url = "http://localhost/version";
        let mut resp = self.agent.get(url).call().ok()?;
        if resp.status().as_u16() != 200 {
            return None;
        }
        let body: VersionResponse = resp.body_mut().read_json().ok()?;
        semver::Version::parse(&body.version).ok()
    }

    fn image_digest(&self, image_ref: &ImageRef) -> Option<ImageDigest> {
        let encoded = url_encode(image_ref.as_str());
        let url = format!("{}/images/{}/json", API_BASE, encoded);
        let mut resp = self.agent.get(&url).call().ok()?;
        if resp.status().as_u16() != 200 {
            return None;
        }
        let body: ImageInspect = resp.body_mut().read_json().ok()?;
        ImageDigest::parse(body.digest).ok()
    }

    // ── Pod operations ────────────────────────────────────────────────

    fn pod_create(&self, name: &str, host_aliases: &[String]) -> Result<PodId, PodmanError> {
        let url = format!("{}/pods/create", API_BASE);
        let hostadd = if host_aliases.is_empty() { None } else { Some(host_aliases.to_vec()) };
        let spec = PodCreateSpec {
            name: name.to_string(),
            hostadd,
        };

        let mut resp = self.agent.post(&url)
            .send_json(&spec)
            .map_err(|e| PodmanError::Run(format!("pod create failed: {}", e)))?;

        let status = resp.status().as_u16();
        if status != 200 && status != 201 {
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(PodmanError::Run(format!("pod create HTTP {}: {}", status, body)));
        }

        let created: PodCreateResponse = resp.body_mut()
            .read_json()
            .map_err(|e| PodmanError::Run(format!("failed to parse pod create response: {}", e)))?;

        Ok(PodId::new(created.id))
    }

    fn container_in_pod(
        &self,
        image: RunTarget<'_>,
        pod_id: &PodId,
        env_vars: &[(&str, &str)],
    ) -> Result<ContainerId, PodmanError> {
        let env: HashMap<String, String> = env_vars.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let spec = PodContainerCreateSpec {
            image: image.as_str().to_string(),
            pod: pod_id.as_str().to_string(),
            env: if env.is_empty() { None } else { Some(env) },
        };

        let url = format!("{}/containers/create", API_BASE);
        let mut resp = self.agent.post(&url)
            .send_json(&spec)
            .map_err(|e| PodmanError::Run(format!("container create in pod failed: {}", e)))?;

        let status = resp.status().as_u16();
        if status != 201 {
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(PodmanError::Run(format!("container create in pod HTTP {}: {}", status, body)));
        }

        let created: ContainerCreateResponse = resp.body_mut()
            .read_json()
            .map_err(|e| PodmanError::Run(format!("failed to parse container create response: {}", e)))?;

        Ok(ContainerId::new(created.id))
    }

    fn pod_start(&self, pod_id: &PodId) -> Result<(), PodmanError> {
        let url = format!("{}/pods/{}/start", API_BASE, pod_id.as_str());
        let resp = self.agent.post(&url)
            .send("")
            .map_err(|e| PodmanError::Run(format!("pod start failed: {}", e)))?;

        let status = resp.status().as_u16();
        if status != 200 && status != 204 && status != 304 {
            return Err(PodmanError::Run(format!("pod start HTTP {}", status)));
        }

        Ok(())
    }

    fn pod_stop_and_remove(&self, pod_id: &PodId, timeout_secs: u32) {
        let id = pod_id.as_str();

        // Stop — fire and forget
        let url = format!("{}/pods/{}/stop?t={}", API_BASE, id, timeout_secs);
        let _ = self.agent.post(&url).send("");

        // Remove with force — fire and forget
        let url = format!("{}/pods/{}?force=true", API_BASE, id);
        let _ = self.agent.delete(&url).call();
    }
}

// ── Request/response serde types ─────────────────────────────────────

#[derive(Deserialize)]
struct VersionResponse {
    #[serde(alias = "Version")]
    version: String,
}

#[derive(Deserialize)]
struct ImageInspect {
    #[serde(alias = "Digest")]
    digest: String,
}

#[derive(Deserialize)]
struct ContainerCreateResponse {
    #[serde(alias = "Id")]
    id: String,
}

#[derive(Serialize)]
struct PodCreateSpec {
    name: String,
    /// Host aliases injected into `/etc/hosts` (`hostname:ip` pairs).
    #[serde(skip_serializing_if = "Option::is_none")]
    hostadd: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct PodCreateResponse {
    #[serde(alias = "Id")]
    id: String,
}

#[derive(Serialize)]
struct PodContainerCreateSpec {
    image: String,
    pod: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<HashMap<String, String>>,
}

// ── Helpers ──────────────────────────────────────────────────────────

/// URL-encode an image reference (e.g. `docker.io/library/nginx` → `docker.io%2Flibrary%2Fnginx`).
fn url_encode(s: &str) -> String {
    s.replace('/', "%2F").replace(':', "%3A")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── URL encoding ──

    #[test]
    fn url_encode_image_ref() {
        assert_eq!(url_encode("docker.io/library/nginx"), "docker.io%2Flibrary%2Fnginx");
    }

    #[test]
    fn url_encode_with_tag() {
        assert_eq!(url_encode("localhost/myapp:latest"), "localhost%2Fmyapp%3Alatest");
    }

    #[test]
    fn url_encode_simple_name() {
        assert_eq!(url_encode("nginx"), "nginx");
    }

    // ── Version response parsing ──

    #[test]
    fn parse_version_response() {
        let json = r#"{"Version":"5.3.1","ApiVersion":"5.3.1"}"#;
        let v: VersionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(v.version, "5.3.1");
    }

    // ── Image inspect response parsing ──

    #[test]
    fn parse_image_inspect_digest() {
        let json = r#"{"Digest":"sha256:abc123def456"}"#;
        let img: ImageInspect = serde_json::from_str(json).unwrap();
        assert_eq!(img.digest, "sha256:abc123def456");
    }

    // ── Container create response parsing ──

    #[test]
    fn parse_container_create_response() {
        let json = r#"{"Id":"abc123","Warnings":[]}"#;
        let resp: ContainerCreateResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "abc123");
    }

    // ── Pod serde types ──

    #[test]
    fn pod_create_spec_serialization() {
        let spec = PodCreateSpec { name: "vlinder-echo".to_string(), hostadd: None };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("vlinder-echo"));
        assert!(!json.contains("hostadd"));
    }

    #[test]
    fn pod_create_spec_with_host_aliases() {
        let spec = PodCreateSpec {
            name: "vlinder-provider-test".to_string(),
            hostadd: Some(vec!["openrouter.vlinder.local:127.0.0.1".to_string()]),
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("openrouter.vlinder.local:127.0.0.1"));
    }

    #[test]
    fn parse_pod_create_response() {
        let json = r#"{"Id":"pod123abc"}"#;
        let resp: PodCreateResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "pod123abc");
    }

    #[test]
    fn pod_container_create_spec_with_env() {
        let mut env = HashMap::new();
        env.insert("KEY".to_string(), "VALUE".to_string());
        let spec = PodContainerCreateSpec {
            image: "localhost/echo:latest".to_string(),
            pod: "pod123".to_string(),
            env: Some(env),
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("KEY"));
        assert!(json.contains("VALUE"));
        assert!(json.contains("pod123"));
    }

    #[test]
    fn pod_container_create_spec_without_env() {
        let spec = PodContainerCreateSpec {
            image: "localhost/echo:latest".to_string(),
            pod: "pod123".to_string(),
            env: None,
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(!json.contains("env"));
    }
}
