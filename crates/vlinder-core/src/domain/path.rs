//! Absolute path types that enforce resolution at compile time.
//!
//! These newtypes ensure paths are resolved to absolute form before
//! being stored in domain types. Invalid states are unrepresentable.

use std::path::{Path, PathBuf};

/// An absolute file:// URI. Can only be created through resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbsoluteUri(String);

impl AbsoluteUri {
    /// Create from a URI that's already absolute.
    /// Returns None if the URI is not absolute.
    pub fn from_absolute(uri: &str) -> Option<Self> {
        if is_absolute_uri(uri) {
            Some(AbsoluteUri(uri.to_string()))
        } else {
            None
        }
    }

    /// Resolve a potentially relative URI against a base directory.
    /// Always produces an absolute URI.
    pub fn resolve(uri: &str, base_dir: &Path) -> Self {
        if is_absolute_uri(uri) {
            return AbsoluteUri(uri.to_string());
        }

        // Handle file:// URIs with relative paths
        if let Some(path) = uri.strip_prefix("file://") {
            let clean_path = path.strip_prefix("./").unwrap_or(path);
            let resolved = base_dir.join(clean_path);
            return AbsoluteUri(format!("file://{}", resolved.display()));
        }

        // Non-file URIs pass through (e.g., ollama://, bedrock://)
        AbsoluteUri(uri.to_string())
    }

    /// Get the URI as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Extract the path portion from a file:// URI.
    pub fn file_path(&self) -> Option<&str> {
        self.0.strip_prefix("file://")
    }
}

impl std::fmt::Display for AbsoluteUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for AbsoluteUri {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// An absolute filesystem path. Can only be created through resolution.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AbsolutePath(PathBuf);

impl AbsolutePath {
    /// Create from a path that's already absolute.
    /// Returns None if the path is not absolute.
    pub fn from_absolute(path: &Path) -> Option<Self> {
        if path.is_absolute() {
            Some(AbsolutePath(path.to_path_buf()))
        } else {
            None
        }
    }

    /// Resolve a potentially relative path against a base directory.
    /// Always produces an absolute path.
    pub fn resolve(path: &str, base_dir: &Path) -> Self {
        let p = Path::new(path);
        if p.is_absolute() {
            AbsolutePath(p.to_path_buf())
        } else {
            AbsolutePath(base_dir.join(path))
        }
    }

    /// Get the path as a `PathBuf` reference.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Check if the path exists on the filesystem.
    pub fn exists(&self) -> bool {
        self.0.exists()
    }
}

impl std::fmt::Display for AbsolutePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl AsRef<Path> for AbsolutePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// Check if a URI is absolute (has scheme and absolute path).
fn is_absolute_uri(uri: &str) -> bool {
    if let Some(path) = uri.strip_prefix("file://") {
        Path::new(path).is_absolute()
    } else if uri.contains("://") {
        // Other schemes (ollama://, bedrock://) are considered "absolute"
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_uri_from_absolute() {
        let uri = AbsoluteUri::from_absolute("file:///absolute/path.toml");
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().as_str(), "file:///absolute/path.toml");
    }

    #[test]
    fn absolute_uri_rejects_relative() {
        let uri = AbsoluteUri::from_absolute("file://./relative/path.toml");
        assert!(uri.is_none());
    }

    #[test]
    fn absolute_uri_resolve_relative() {
        let base = Path::new("/agent/dir");
        let uri = AbsoluteUri::resolve("file://./models/phi3.toml", base);
        assert_eq!(uri.as_str(), "file:///agent/dir/models/phi3.toml");
    }

    #[test]
    fn absolute_uri_resolve_already_absolute() {
        let base = Path::new("/agent/dir");
        let uri = AbsoluteUri::resolve("file:///absolute/path.toml", base);
        assert_eq!(uri.as_str(), "file:///absolute/path.toml");
    }

    #[test]
    fn absolute_uri_non_file_scheme() {
        let base = Path::new("/agent/dir");
        let uri = AbsoluteUri::resolve("ollama://phi3", base);
        assert_eq!(uri.as_str(), "ollama://phi3");
    }

    #[test]
    fn absolute_path_from_absolute() {
        let path = AbsolutePath::from_absolute(Path::new("/absolute/path"));
        assert!(path.is_some());
    }

    #[test]
    fn absolute_path_rejects_relative() {
        let path = AbsolutePath::from_absolute(Path::new("relative/path"));
        assert!(path.is_none());
    }

    #[test]
    fn absolute_path_resolve_relative() {
        let base = Path::new("/agent/dir");
        let path = AbsolutePath::resolve("mnt/data", base);
        assert_eq!(path.as_path(), Path::new("/agent/dir/mnt/data"));
    }
}
