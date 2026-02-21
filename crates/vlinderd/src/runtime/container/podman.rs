//! Podman abstraction — trait + shared utilities.
//!
//! The `Podman` trait is the contract for all container engine interactions.
//! Two implementations exist:
//! - `PodmanApiClient` (primary) — REST API over Unix socket
//! - `PodmanCliClient` (fallback) — shells out to the `podman` binary

use std::fmt;

use crate::domain::{ContainerId, ImageDigest, ImageRef};

// ── Error type ──────────────────────────────────────────────────────

/// Podman operation failure.
///
/// Three variants match the three fallible phases of container startup:
/// run, port discovery, and health check.
#[derive(Debug)]
pub(crate) enum PodmanError {
    /// Container create or start failed.
    Run(String),
    /// Host port could not be determined.
    Port(String),
    /// Container health check timed out.
    ReadinessTimeout,
}

impl fmt::Display for PodmanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PodmanError::Run(msg) => write!(f, "container run failed: {}", msg),
            PodmanError::Port(msg) => write!(f, "port discovery failed: {}", msg),
            PodmanError::ReadinessTimeout => {
                write!(f, "container did not become ready within 30 seconds")
            }
        }
    }
}

// ── Run target ──────────────────────────────────────────────────────

/// What to pass to `podman run` as the image argument.
///
/// Mutable policy uses the image ref (tag-based, picks up rebuilds).
/// Pinned policy uses the content-addressed digest (deterministic bytes).
pub(crate) enum RunTarget<'a> {
    /// Tag-based image reference (e.g., `localhost/echo:latest`).
    Ref(&'a ImageRef),
    /// Content-addressed digest (e.g., `sha256:abc123...`).
    Digest(&'a ImageDigest),
}

impl RunTarget<'_> {
    /// The string to pass to Podman (CLI flag or API field).
    pub(crate) fn as_str(&self) -> &str {
        match self {
            RunTarget::Ref(r) => r.as_str(),
            RunTarget::Digest(d) => d.as_str(),
        }
    }
}

// ── Trait ────────────────────────────────────────────────────────────

/// Abstraction over the Podman container engine.
///
/// Each method maps to one Podman operation.  The trait is object-safe
/// so `ContainerPool` can hold a `Box<dyn Podman>`.
pub(crate) trait Podman: Send {
    /// Engine version (e.g. 4.9.3).  None if Podman is unavailable.
    fn engine_version(&self) -> Option<semver::Version>;

    /// Start a detached container and return its ID.
    fn run(&self, image: RunTarget<'_>) -> Result<ContainerId, PodmanError>;

    /// Return the content-addressed digest for an image.
    fn image_digest(&self, image_ref: &ImageRef) -> Option<ImageDigest>;

    /// Discover the host port mapped to container port 8080.
    fn port(&self, container_id: &ContainerId) -> Result<u16, PodmanError>;

    /// Tear down a container (stop + force remove).
    fn stop_and_remove(&self, container_id: &ContainerId, timeout_secs: u32);

    /// Poll `GET /health` until the container responds or a deadline expires.
    fn wait_for_ready(&self, host_port: u16) -> Result<(), PodmanError>;
}

// ── Shared utilities ────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;

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
        let result = resolve_socket("auto");
        let _ = result;
    }

    #[test]
    fn run_target_ref() {
        let r = ImageRef::parse("localhost/echo:latest").unwrap();
        let target = RunTarget::Ref(&r);
        assert_eq!(target.as_str(), "localhost/echo:latest");
    }

    #[test]
    fn run_target_digest() {
        let d = ImageDigest::parse("sha256:abc123").unwrap();
        let target = RunTarget::Digest(&d);
        assert_eq!(target.as_str(), "sha256:abc123");
    }

    #[test]
    fn podman_error_display() {
        assert_eq!(
            PodmanError::Run("boom".to_string()).to_string(),
            "container run failed: boom"
        );
        assert_eq!(
            PodmanError::Port("no port".to_string()).to_string(),
            "port discovery failed: no port"
        );
        assert_eq!(
            PodmanError::ReadinessTimeout.to_string(),
            "container did not become ready within 30 seconds"
        );
    }
}
