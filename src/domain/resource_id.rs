//! Resource identifier for registry lookups.
//!
//! ResourceId is a URI that identifies any resource in the registry:
//! storage, models, runtimes, queues, etc.

use serde::Deserialize;

/// A URI that identifies a resource in the registry.
///
/// ResourceId is the key for looking up provisioned resources.
/// The URI scheme indicates the resource type:
/// - `sqlite:///path/to/db.sqlite` → SQLite storage
/// - `s3://bucket/prefix` → S3 storage
/// - `memory://name` → in-memory (testing)
/// - `ollama://phi3` → Ollama model
/// - `file:///path/to/model.gguf` → local model file
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct ResourceId(String);

impl ResourceId {
    /// Create a new ResourceId from a URI string.
    pub fn new(uri: impl Into<String>) -> Self {
        ResourceId(uri.into())
    }

    /// Get the URI as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the scheme (e.g., "sqlite", "s3", "memory", "ollama").
    pub fn scheme(&self) -> Option<&str> {
        self.0.split("://").next()
    }
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ResourceId {
    fn from(s: &str) -> Self {
        ResourceId(s.to_string())
    }
}

impl From<String> for ResourceId {
    fn from(s: String) -> Self {
        ResourceId(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn resource_id_creation() {
        let id = ResourceId::new("sqlite:///path/to/db.sqlite");
        assert_eq!(id.as_str(), "sqlite:///path/to/db.sqlite");
    }

    #[test]
    fn resource_id_scheme() {
        let sqlite = ResourceId::new("sqlite:///path/to/db");
        assert_eq!(sqlite.scheme(), Some("sqlite"));

        let s3 = ResourceId::new("s3://bucket/prefix");
        assert_eq!(s3.scheme(), Some("s3"));

        let memory = ResourceId::new("memory://test");
        assert_eq!(memory.scheme(), Some("memory"));

        let ollama = ResourceId::new("ollama://phi3");
        assert_eq!(ollama.scheme(), Some("ollama"));
    }

    #[test]
    fn resource_id_from_str() {
        let id: ResourceId = "memory://test".into();
        assert_eq!(id.as_str(), "memory://test");
    }

    #[test]
    fn resource_id_hashable() {
        let mut map: HashMap<ResourceId, &str> = HashMap::new();
        map.insert(ResourceId::new("sqlite:///a"), "first");
        map.insert(ResourceId::new("sqlite:///b"), "second");

        assert_eq!(map.get(&ResourceId::new("sqlite:///a")), Some(&"first"));
        assert_eq!(map.get(&ResourceId::new("sqlite:///b")), Some(&"second"));
    }

    #[test]
    fn resource_id_display() {
        let id = ResourceId::new("s3://bucket/key");
        assert_eq!(format!("{}", id), "s3://bucket/key");
    }

    #[test]
    fn resource_id_deserialize() {
        let json = r#""memory://test""#;
        let id: ResourceId = serde_json::from_str(json).unwrap();
        assert_eq!(id.as_str(), "memory://test");
    }
}
