//! Podman CLI abstraction — trait + implementation.
//!
//! Every interaction with the `podman` binary lives here.  The `Podman` trait
//! makes the CLI-to-socket-API migration a single-file change and allows unit
//! tests to mock the engine without a real Podman install.

use std::process::Command;

use crate::domain::ImageDigest;

/// Abstraction over the Podman container engine.
///
/// Each method maps to one `podman` CLI subcommand.  The trait is
/// object-safe so `ContainerRuntime` can hold a `Box<dyn Podman>`.
pub(crate) trait Podman: Send {
    /// Engine version (e.g. 4.9.3).  None if Podman is unavailable.
    fn engine_version(&self) -> Option<semver::Version>;

    /// `podman run -d` — start a detached container and return its ID.
    fn run(&self, image: &str, mounts: &[String]) -> Result<String, String>;

    /// `podman image inspect` — return the content-addressed digest.
    fn image_digest(&self, image_ref: &str) -> Option<ImageDigest>;

    /// `podman port` — discover the host port mapped to container port 8080.
    fn port(&self, container_id: &str) -> Result<u16, String>;

    /// `podman stop` + `podman rm -f` — tear down a container.
    fn stop_and_remove(&self, container_id: &str, timeout_secs: u32);

    /// Poll `GET /health` until the container responds or a deadline expires.
    fn wait_for_ready(&self, host_port: u16) -> Result<(), String>;
}

/// Production implementation that shells out to the `podman` CLI.
pub(crate) struct PodmanCli;

impl Podman for PodmanCli {
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

    fn run(&self, image: &str, mounts: &[String]) -> Result<String, String> {
        let mut podman_args = vec![
            "run", "-d",
            "--pull=never",
            "-p", ":8080",
        ];

        for flag in mounts {
            podman_args.push("-v");
            podman_args.push(flag);
        }

        podman_args.push(image);

        let output = Command::new("podman")
            .args(&podman_args)
            .output()
            .map_err(|e| format!("failed to spawn podman: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("podman run failed: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn image_digest(&self, image_ref: &str) -> Option<ImageDigest> {
        Command::new("podman")
            .args(["image", "inspect", image_ref, "--format", "{{.Digest}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .and_then(|s| ImageDigest::parse(s).ok())
    }

    fn port(&self, container_id: &str) -> Result<u16, String> {
        let output = Command::new("podman")
            .args(["port", container_id, "8080"])
            .output()
            .map_err(|e| format!("podman port failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("podman port failed: {}", stderr));
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        parse_port_output(raw.trim())
    }

    fn stop_and_remove(&self, container_id: &str, timeout_secs: u32) {
        let timeout = timeout_secs.to_string();
        let _ = Command::new("podman")
            .args(["stop", "-t", &timeout, container_id])
            .output();
        let _ = Command::new("podman")
            .args(["rm", "-f", container_id])
            .output();
    }

    fn wait_for_ready(&self, host_port: u16) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}/health", host_port);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

        loop {
            if std::time::Instant::now() > deadline {
                return Err("container did not become ready within 30 seconds".to_string());
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

/// Resolve the content-addressed digest for an image via `podman image inspect`.
/// Returns None if the inspect fails (image not found, Podman unavailable, etc.).
pub(crate) fn resolve_image_digest(image_ref: &str) -> Option<ImageDigest> {
    PodmanCli.image_digest(image_ref)
}

/// Resolve a Podman socket path from the config value (ADR 077).
///
/// - `"disabled"` → None (force CLI mode)
/// - `"auto"` → probe standard paths in order, return first that exists
/// - anything else → treat as explicit path
pub(crate) fn resolve_socket(configured: &str) -> Option<std::path::PathBuf> {
    match configured {
        "disabled" => None,
        "auto" => probe_socket_paths(),
        path => {
            let p = std::path::PathBuf::from(path);
            if p.exists() { Some(p) } else { None }
        }
    }
}

/// Probe standard Podman socket locations.
fn probe_socket_paths() -> Option<std::path::PathBuf> {
    // 1. $XDG_RUNTIME_DIR/podman/podman.sock (standard rootless)
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let p = std::path::PathBuf::from(xdg).join("podman/podman.sock");
        if p.exists() {
            return Some(p);
        }
    }

    // 2. macOS Podman Machine socket
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".local/share/containers/podman/machine/podman.sock");
        if p.exists() {
            return Some(p);
        }
    }

    // 3. /run/podman/podman.sock (rootful)
    let p = std::path::PathBuf::from("/run/podman/podman.sock");
    if p.exists() {
        return Some(p);
    }

    None
}

// ── Pure parsing helpers (unit-testable without Podman) ──────────────

/// Parse a semver version string. Returns None on invalid input.
fn parse_version(raw: &str) -> Option<semver::Version> {
    semver::Version::parse(raw).ok()
}

/// Extract host port from `podman port` output.
///
/// Expected format: `"0.0.0.0:XXXXX"` or `"[::]:XXXXX"`.
fn parse_port_output(raw: &str) -> Result<u16, String> {
    raw.rsplit(':')
        .next()
        .ok_or_else(|| format!("unexpected podman port output: {}", raw))?
        .parse::<u16>()
        .map_err(|e| format!("invalid port number: {}", e))
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
        // Some distros ship e.g. 5.0.0-rc1
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
        assert_eq!(parse_port_output("0.0.0.0:43210"), Ok(43210));
    }

    #[test]
    fn parse_port_ipv6() {
        assert_eq!(parse_port_output("[::]:12345"), Ok(12345));
    }

    #[test]
    fn parse_port_bare_number() {
        // Defensive: if Podman ever returns just the port number
        assert_eq!(parse_port_output("8080"), Ok(8080));
    }

    #[test]
    fn parse_port_bad_number() {
        assert!(parse_port_output("0.0.0.0:notaport").is_err());
    }

    #[test]
    fn parse_port_empty() {
        assert!(parse_port_output("").is_err());
    }

    // ── resolve_socket ──

    #[test]
    fn resolve_socket_disabled() {
        assert_eq!(resolve_socket("disabled"), None);
    }

    #[test]
    fn resolve_socket_explicit_path_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sock = tmp.path().join("podman.sock");
        std::fs::write(&sock, "").unwrap();
        assert_eq!(resolve_socket(sock.to_str().unwrap()), Some(sock));
    }

    #[test]
    fn resolve_socket_explicit_path_missing() {
        assert_eq!(resolve_socket("/nonexistent/path/podman.sock"), None);
    }

    #[test]
    fn resolve_socket_auto_no_sockets() {
        // On CI / dev machines without Podman, auto should return None
        // (unless a real socket exists, in which case it returns Some)
        let result = resolve_socket("auto");
        // Just verify it doesn't panic — the result depends on the host
        let _ = result;
    }
}
