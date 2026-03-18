//! Resource identifier for registry lookups.
//!
//! `ResourceId` is a URI that identifies any resource in the registry:
//! storage, models, runtimes, queues, etc.

use serde::{Deserialize, Serialize};

/// A URI that identifies a resource in the registry.
///
/// `ResourceId` is the key for looking up provisioned resources.
/// The URI scheme indicates the resource type:
/// - `sqlite:///path/to/db.sqlite` → `SQLite` storage
/// - `s3://bucket/prefix` → S3 storage
/// - `memory://name` → in-memory (testing)
/// - `ollama://phi3` → Ollama model
/// - `file:///path/to/model.gguf` → local model file
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResourceId(String);

impl ResourceId {
    /// Create a new `ResourceId` from a URI string.
    pub fn new(uri: impl Into<String>) -> Self {
        ResourceId(uri.into())
    }

    /// Get the URI as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the scheme (e.g., "sqlite", "s3", "memory", "ollama").
    pub fn scheme(&self) -> Option<&str> {
        if !self.0.contains("://") {
            return None;
        }
        self.0.split("://").next()
    }

    /// Get the authority (host, bucket, or service name).
    ///
    /// URI format: `scheme://authority/path`
    /// - `s3://bucket/key` → Some("bucket")
    /// - `https://api.example.com/path` → Some("api.example.com")
    /// - `file:///absolute/path` → None (empty authority)
    /// - `memory://test` → Some("test")
    pub fn authority(&self) -> Option<&str> {
        // Get everything after "://"
        let after_scheme = self.0.split("://").nth(1)?;

        // Split on first "/" to separate authority from path
        let authority = match after_scheme.find('/') {
            Some(0) => return None, // Empty authority (e.g., file:///)
            Some(idx) => &after_scheme[..idx],
            None => after_scheme, // No path, entire rest is authority
        };

        if authority.is_empty() {
            None
        } else {
            Some(authority)
        }
    }

    /// Get the path component.
    ///
    /// URI format: `scheme://authority/path`
    /// - `s3://bucket/prefix/key` → Some("/prefix/key")
    /// - `file:///home/user/file.txt` → Some("/home/user/file.txt")
    /// - `memory://test` → None (no path)
    pub fn path(&self) -> Option<&str> {
        // Get everything after "://"
        let after_scheme = self.0.split("://").nth(1)?;

        // Find the first "/" which starts the path
        let path_start = after_scheme.find('/')?;
        let path = &after_scheme[path_start..];

        if path.is_empty() || path == "/" {
            None
        } else {
            Some(path)
        }
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

    // ========================================================================
    // URI Parsing Tests (TDD - tests first, implementation follows)
    // ========================================================================

    #[test]
    fn authority_for_s3() {
        // s3://bucket/prefix/key → authority is "bucket"
        let id = ResourceId::new("s3://my-bucket/prefix/key");
        assert_eq!(id.authority(), Some("my-bucket"));
    }

    #[test]
    fn authority_for_https() {
        // https://api.example.com/v1/agents → authority is "api.example.com"
        let id = ResourceId::new("https://api.example.com/v1/agents");
        assert_eq!(id.authority(), Some("api.example.com"));
    }

    #[test]
    fn authority_for_memory() {
        // memory://test → authority is "test"
        let id = ResourceId::new("memory://test");
        assert_eq!(id.authority(), Some("test"));
    }

    #[test]
    fn authority_for_file_is_empty() {
        // file:///absolute/path → no authority (empty string between file:// and /path)
        let id = ResourceId::new("file:///home/user/agent.wasm");
        assert_eq!(id.authority(), None);
    }

    #[test]
    fn authority_for_sqlite_is_empty() {
        // sqlite:///path/to/db.sqlite → no authority
        let id = ResourceId::new("sqlite:///data/notes.db");
        assert_eq!(id.authority(), None);
    }

    #[test]
    fn path_for_s3() {
        // s3://bucket/prefix/key → path is "/prefix/key"
        let id = ResourceId::new("s3://my-bucket/prefix/key");
        assert_eq!(id.path(), Some("/prefix/key"));
    }

    #[test]
    fn path_for_file() {
        // file:///home/user/agent.wasm → path is "/home/user/agent.wasm"
        let id = ResourceId::new("file:///home/user/agent.wasm");
        assert_eq!(id.path(), Some("/home/user/agent.wasm"));
    }

    #[test]
    fn path_for_memory_is_empty() {
        // memory://test → no path
        let id = ResourceId::new("memory://test");
        assert_eq!(id.path(), None);
    }

    #[test]
    fn path_for_https() {
        // https://api.example.com/v1/agents → path is "/v1/agents"
        let id = ResourceId::new("https://api.example.com/v1/agents");
        assert_eq!(id.path(), Some("/v1/agents"));
    }

    #[test]
    fn path_for_ollama_is_empty() {
        // ollama://phi3 → no path
        let id = ResourceId::new("ollama://phi3");
        assert_eq!(id.path(), None);
    }

    #[test]
    fn scheme_returns_none_for_invalid() {
        // No "://" separator
        let id = ResourceId::new("not-a-uri");
        assert_eq!(id.scheme(), None);
    }
}
