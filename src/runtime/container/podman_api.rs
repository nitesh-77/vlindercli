//! PodmanApiClient — Podman trait implementation using the libpod REST API.
//!
//! Primary implementation. Talks to Podman's REST API over a Unix socket.
//! Uses ureq with a Unix transport adapter for HTTP-over-socket.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::{ContainerId, ImageDigest, ImageRef, Mount};

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

    fn run(&self, image: RunTarget<'_>, mounts: &[Mount]) -> Result<ContainerId, PodmanError> {
        // Build the container spec
        let port_mapping = vec![PortMapping {
            container_port: 8080,
            host_port: 0,
            protocol: "tcp".to_string(),
        }];

        // Convert domain::Mount directly to API mount structs — no string round-trip
        let api_mounts: Vec<ApiMount> = mounts.iter().map(|m| {
            let mode = if m.readonly { "ro" } else { "rw" };
            ApiMount {
                source: m.host_path.to_string(),
                destination: m.guest_path.display().to_string(),
                mount_type: "bind".to_string(),
                options: vec![mode.to_string()],
            }
        }).collect();

        let spec = ContainerCreateSpec {
            image: image.as_str().to_string(),
            portmappings: Some(port_mapping),
            mounts: if api_mounts.is_empty() { None } else { Some(api_mounts) },
        };

        // Create container
        let url = format!("{}/containers/create", API_BASE);
        let mut resp = self.agent.post(&url)
            .send_json(&spec)
            .map_err(|e| PodmanError::Run(format!("container create failed: {}", e)))?;

        let status = resp.status().as_u16();
        if status != 201 {
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(PodmanError::Run(format!("container create HTTP {}: {}", status, body)));
        }

        let created: ContainerCreateResponse = resp.body_mut()
            .read_json()
            .map_err(|e| PodmanError::Run(format!("failed to parse create response: {}", e)))?;

        let container_id = created.id;

        // Start container
        let url = format!("{}/containers/{}/start", API_BASE, container_id);
        let resp = self.agent.post(&url)
            .send("")
            .map_err(|e| PodmanError::Run(format!("container start failed: {}", e)))?;

        let status = resp.status().as_u16();
        if status != 204 && status != 304 {
            return Err(PodmanError::Run(format!("container start HTTP {}", status)));
        }

        Ok(ContainerId::new(container_id))
    }

    fn port(&self, container_id: &ContainerId) -> Result<u16, PodmanError> {
        let url = format!("{}/containers/{}/json", API_BASE, container_id.as_str());
        let mut resp = self.agent.get(&url)
            .call()
            .map_err(|e| PodmanError::Port(format!("container inspect failed: {}", e)))?;

        let status = resp.status().as_u16();
        if status != 200 {
            return Err(PodmanError::Port(format!("container inspect HTTP {}", status)));
        }

        let body: ContainerInspect = resp.body_mut()
            .read_json()
            .map_err(|e| PodmanError::Port(format!("failed to parse inspect response: {}", e)))?;

        let ports = body.network_settings
            .and_then(|ns| ns.ports)
            .ok_or_else(|| PodmanError::Port("no port mappings in container inspect".to_string()))?;

        let bindings = ports.get("8080/tcp")
            .ok_or_else(|| PodmanError::Port("port 8080/tcp not found in container".to_string()))?;

        bindings.first()
            .and_then(|b| b.host_port.parse::<u16>().ok())
            .ok_or_else(|| PodmanError::Port("no host port binding for 8080".to_string()))
    }

    fn stop_and_remove(&self, container_id: &ContainerId, timeout_secs: u32) {
        let id = container_id.as_str();

        // Stop — fire and forget
        let url = format!("{}/containers/{}/stop?t={}", API_BASE, id, timeout_secs);
        let _ = self.agent.post(&url).send("");

        // Remove with force — fire and forget
        let url = format!("{}/containers/{}?force=true", API_BASE, id);
        let _ = self.agent.delete(&url).call();
    }

    fn wait_for_ready(&self, host_port: u16) -> Result<(), PodmanError> {
        let url = format!("http://127.0.0.1:{}/health", host_port);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

        loop {
            if std::time::Instant::now() > deadline {
                return Err(PodmanError::ReadinessTimeout);
            }

            match ureq::get(&url).call() {
                Ok(_) => return Ok(()),
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
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

#[derive(Serialize)]
struct ContainerCreateSpec {
    image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    portmappings: Option<Vec<PortMapping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mounts: Option<Vec<ApiMount>>,
}

#[derive(Serialize)]
struct PortMapping {
    container_port: u16,
    host_port: u16,
    protocol: String,
}

/// API-specific mount struct for the Podman REST API JSON body.
/// Distinct from domain::Mount — this is a wire format, not a domain type.
#[derive(Serialize)]
struct ApiMount {
    destination: String,
    source: String,
    #[serde(rename = "type")]
    mount_type: String,
    options: Vec<String>,
}

#[derive(Deserialize)]
struct ContainerCreateResponse {
    #[serde(alias = "Id")]
    id: String,
}

#[derive(Deserialize)]
struct ContainerInspect {
    #[serde(alias = "NetworkSettings")]
    network_settings: Option<NetworkSettings>,
}

#[derive(Deserialize)]
struct NetworkSettings {
    #[serde(alias = "Ports")]
    ports: Option<HashMap<String, Vec<PortBinding>>>,
}

#[derive(Deserialize)]
struct PortBinding {
    #[serde(alias = "HostPort")]
    host_port: String,
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

    // ── Container inspect response parsing ──

    #[test]
    fn parse_container_inspect_ports() {
        let json = r#"{
            "NetworkSettings": {
                "Ports": {
                    "8080/tcp": [{"HostIp": "", "HostPort": "43210"}]
                }
            }
        }"#;
        let inspect: ContainerInspect = serde_json::from_str(json).unwrap();
        let ports = inspect.network_settings.unwrap().ports.unwrap();
        let binding = &ports["8080/tcp"][0];
        assert_eq!(binding.host_port, "43210");
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
}
