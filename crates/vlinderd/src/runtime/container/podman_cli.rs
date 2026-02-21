//! PodmanCliClient — Podman trait implementation via the `podman` CLI.
//!
//! Fallback implementation that shells out to the `podman` binary.
//! Used when no Podman socket is available (ADR 077).

use std::process::Command;

use crate::domain::{ContainerId, ImageDigest, ImageRef};

use super::podman::{Podman, PodmanError, RunTarget};

/// Fallback implementation that shells out to the `podman` CLI.
pub(crate) struct PodmanCliClient;

impl Podman for PodmanCliClient {
    fn engine_version(&self) -> Option<semver::Version> {
        Command::new("podman")
            .args(["version", "--format", "{{.Client.Version}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                let raw = String::from_utf8_lossy(&o.stdout).trim().to_string();
                parse_version(&raw)
            })
    }

    fn run(&self, image: RunTarget<'_>) -> Result<ContainerId, PodmanError> {
        let mut podman_args = vec![
            "run".to_string(), "-d".to_string(),
            "--pull=never".to_string(),
            "-p".to_string(), ":8080".to_string(),
        ];

        podman_args.push(image.as_str().to_string());

        let output = Command::new("podman")
            .args(&podman_args)
            .output()
            .map_err(|e| PodmanError::Run(format!("failed to spawn podman: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PodmanError::Run(format!("podman run failed: {}", stderr)));
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(ContainerId::new(raw))
    }

    fn image_digest(&self, image_ref: &ImageRef) -> Option<ImageDigest> {
        Command::new("podman")
            .args(["image", "inspect", image_ref.as_str(), "--format", "{{.Digest}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .and_then(|s| ImageDigest::parse(s).ok())
    }

    fn port(&self, container_id: &ContainerId) -> Result<u16, PodmanError> {
        let output = Command::new("podman")
            .args(["port", container_id.as_str(), "8080"])
            .output()
            .map_err(|e| PodmanError::Port(format!("podman port failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PodmanError::Port(format!("podman port failed: {}", stderr)));
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        parse_port_output(raw.trim())
    }

    fn stop_and_remove(&self, container_id: &ContainerId, timeout_secs: u32) {
        let timeout = timeout_secs.to_string();
        let id = container_id.as_str();
        let _ = Command::new("podman")
            .args(["stop", "-t", &timeout, id])
            .output();
        let _ = Command::new("podman")
            .args(["rm", "-f", id])
            .output();
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

// ── Pure parsing helpers (unit-testable without Podman) ──────────────

/// Parse a semver version string. Returns None on invalid input.
fn parse_version(raw: &str) -> Option<semver::Version> {
    semver::Version::parse(raw).ok()
}

/// Extract host port from `podman port` output.
///
/// Expected format: `"0.0.0.0:XXXXX"` or `"[::]:XXXXX"`.
fn parse_port_output(raw: &str) -> Result<u16, PodmanError> {
    raw.rsplit(':')
        .next()
        .ok_or_else(|| PodmanError::Port(format!("unexpected podman port output: {}", raw)))?
        .parse::<u16>()
        .map_err(|e| PodmanError::Port(format!("invalid port number: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_version ──

    #[test]
    fn parse_version_valid() {
        assert_eq!(
            parse_version("4.9.3"),
            Some(semver::Version::new(4, 9, 3))
        );
    }

    #[test]
    fn parse_version_with_pre() {
        assert!(parse_version("5.0.0-rc1").is_some());
    }

    #[test]
    fn parse_version_garbage() {
        assert_eq!(parse_version("not-a-version"), None);
    }

    #[test]
    fn parse_version_empty() {
        assert_eq!(parse_version(""), None);
    }

    // ── parse_port_output ──

    #[test]
    fn parse_port_ipv4() {
        assert!(matches!(parse_port_output("0.0.0.0:43210"), Ok(43210)));
    }

    #[test]
    fn parse_port_ipv6() {
        assert!(matches!(parse_port_output("[::]:12345"), Ok(12345)));
    }

    #[test]
    fn parse_port_bare_number() {
        assert!(matches!(parse_port_output("8080"), Ok(8080)));
    }

    #[test]
    fn parse_port_bad_number() {
        assert!(matches!(parse_port_output("0.0.0.0:notaport"), Err(PodmanError::Port(_))));
    }

    #[test]
    fn parse_port_empty() {
        assert!(matches!(parse_port_output(""), Err(PodmanError::Port(_))));
    }
}
