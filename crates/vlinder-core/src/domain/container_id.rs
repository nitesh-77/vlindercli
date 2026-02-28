//! Podman container ID newtype.
//!
//! Wraps the hex string returned by `podman run` or the container API.
//! No format validation — Podman returns variable-length hex IDs and
//! the placeholder `"unknown"` is used in diagnostics. The value is
//! type safety, not input validation.

use serde::{Deserialize, Serialize};

/// A Podman container ID (e.g., `"abc123def456"`).
///
/// Simple newtype for type safety — prevents accidentally passing
/// a container ID where an image ref or pod name is expected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerId(String);

impl ContainerId {
    /// Wrap a raw container ID string.
    pub fn new(id: impl Into<String>) -> Self {
        ContainerId(id.into())
    }

    /// Placeholder for diagnostics when no real container ID is available.
    pub fn unknown() -> Self {
        ContainerId("unknown".to_string())
    }

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContainerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ContainerId> for String {
    fn from(id: ContainerId) -> Self {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_wraps_string() {
        let id = ContainerId::new("abc123def456");
        assert_eq!(id.as_str(), "abc123def456");
    }

    #[test]
    fn unknown_placeholder() {
        let id = ContainerId::unknown();
        assert_eq!(id.as_str(), "unknown");
    }

    #[test]
    fn display_delegates_to_inner() {
        let id = ContainerId::new("abc123");
        assert_eq!(format!("{}", id), "abc123");
    }

    #[test]
    fn into_string() {
        let id = ContainerId::new("abc123");
        let s: String = id.into();
        assert_eq!(s, "abc123");
    }

    #[test]
    fn json_round_trip() {
        let id = ContainerId::new("abc123def456");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#""abc123def456""#);
        let back: ContainerId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "abc123def456");
    }

    #[test]
    fn toml_round_trip() {
        // ContainerId appears inside structs (e.g., RuntimeInfo::Container),
        // so test via a wrapper — TOML requires a top-level table.
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Wrapper { id: ContainerId }

        let w = Wrapper { id: ContainerId::new("abc123") };
        let toml_str = toml::to_string(&w).unwrap();
        let back: Wrapper = toml::from_str(&toml_str).unwrap();
        assert_eq!(w, back);
    }

    #[test]
    fn equality() {
        assert_eq!(ContainerId::new("abc"), ContainerId::new("abc"));
        assert_ne!(ContainerId::new("abc"), ContainerId::new("def"));
    }
}
