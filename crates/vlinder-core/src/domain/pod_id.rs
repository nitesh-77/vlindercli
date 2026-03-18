//! Podman pod ID newtype.
//!
//! Wraps the hex string returned by `podman pod create` or the pod API.
//! Same pattern as `ContainerId` — type safety, not input validation.

use serde::{Deserialize, Serialize};

/// A Podman pod ID (e.g., `"abc123def456"`).
///
/// Simple newtype for type safety — prevents accidentally passing
/// a pod ID where a container ID or image ref is expected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodId(String);

impl PodId {
    /// Wrap a raw pod ID string.
    pub fn new(id: impl Into<String>) -> Self {
        PodId(id.into())
    }

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PodId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<PodId> for String {
    fn from(id: PodId) -> Self {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_wraps_string() {
        let id = PodId::new("abc123def456");
        assert_eq!(id.as_str(), "abc123def456");
    }

    #[test]
    fn display_delegates_to_inner() {
        let id = PodId::new("abc123");
        assert_eq!(format!("{id}"), "abc123");
    }

    #[test]
    fn into_string() {
        let id = PodId::new("abc123");
        let s: String = id.into();
        assert_eq!(s, "abc123");
    }

    #[test]
    fn json_round_trip() {
        let id = PodId::new("abc123def456");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#""abc123def456""#);
        let back: PodId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "abc123def456");
    }

    #[test]
    fn equality() {
        assert_eq!(PodId::new("abc"), PodId::new("abc"));
        assert_ne!(PodId::new("abc"), PodId::new("def"));
    }
}
