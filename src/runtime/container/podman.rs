//! Podman abstraction — trait + shared utilities.
//!
//! The `Podman` trait is the contract for all container engine interactions.
//! Two implementations exist:
//! - `PodmanApiClient` (primary) — REST API over Unix socket
//! - `PodmanCliClient` (fallback) — shells out to the `podman` binary

use crate::domain::ImageDigest;

use super::podman_cli::PodmanCliClient;

/// Abstraction over the Podman container engine.
///
/// Each method maps to one Podman operation.  The trait is object-safe
/// so `ContainerPool` can hold a `Box<dyn Podman>`.
pub(crate) trait Podman: Send {
    /// Engine version (e.g. 4.9.3).  None if Podman is unavailable.
    fn engine_version(&self) -> Option<semver::Version>;

    /// Start a detached container and return its ID.
    fn run(&self, image: &str, mounts: &[String]) -> Result<String, String>;

    /// Return the content-addressed digest for an image.
    fn image_digest(&self, image_ref: &str) -> Option<ImageDigest>;

    /// Discover the host port mapped to container port 8080.
    fn port(&self, container_id: &str) -> Result<u16, String>;

    /// Tear down a container (stop + force remove).
    fn stop_and_remove(&self, container_id: &str, timeout_secs: u32);

    /// Poll `GET /health` until the container responds or a deadline expires.
    fn wait_for_ready(&self, host_port: u16) -> Result<(), String>;
}

// ── Shared utilities ────────────────────────────────────────────────

/// Resolve the content-addressed digest for an image via `podman image inspect`.
/// Returns None if the inspect fails (image not found, Podman unavailable, etc.).
pub(crate) fn resolve_image_digest(image_ref: &str) -> Option<ImageDigest> {
    PodmanCliClient.image_digest(image_ref)
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
        // On CI / dev machines without Podman, auto should return None
        // (unless a real socket exists, in which case it returns Some)
        let result = resolve_socket("auto");
        // Just verify it doesn't panic — the result depends on the host
        let _ = result;
    }
}
