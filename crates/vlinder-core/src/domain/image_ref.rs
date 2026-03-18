//! OCI image reference newtype.
//!
//! Wraps an image reference string (e.g., `localhost/echo:latest`) with
//! parse-time validation. Full OCI reference parsing (registry, repository,
//! tag, digest components) is deferred — the newtype gives type safety now.

use serde::{Deserialize, Serialize};

/// A validated OCI image reference (e.g., `localhost/echo-agent:latest`).
///
/// Validation rules:
/// - Must not be empty
/// - Must not contain whitespace
/// - Must contain at least one `/` (registry/repo separator)
///
/// All OCI refs in this codebase use `localhost/name:tag` form, so the
/// slash requirement catches bare names like `"echo:latest"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ImageRef(String);

impl ImageRef {
    /// Validated constructor.
    ///
    /// Returns `Err` if the input is empty, contains whitespace, or
    /// lacks at least one `/`.
    pub fn parse(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();

        if s.is_empty() {
            return Err("image ref must not be empty".to_string());
        }

        if s.chars().any(char::is_whitespace) {
            return Err(format!("image ref must not contain whitespace: {s}"));
        }

        if !s.contains('/') {
            return Err(format!("image ref must contain at least one '/': {s}"));
        }

        Ok(ImageRef(s))
    }

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ImageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ImageRef> for String {
    fn from(r: ImageRef) -> Self {
        r.0
    }
}

impl TryFrom<String> for ImageRef {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        ImageRef::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Valid refs ──

    #[test]
    fn parse_localhost_tag() {
        let r = ImageRef::parse("localhost/echo:latest").unwrap();
        assert_eq!(r.as_str(), "localhost/echo:latest");
    }

    #[test]
    fn parse_registry_with_org() {
        let r = ImageRef::parse("registry.io/org/repo:v1.2.3").unwrap();
        assert_eq!(r.as_str(), "registry.io/org/repo:v1.2.3");
    }

    #[test]
    fn parse_ref_with_digest() {
        let r = ImageRef::parse("localhost/app@sha256:abc123").unwrap();
        assert_eq!(r.as_str(), "localhost/app@sha256:abc123");
    }

    // ── Invalid refs ──

    #[test]
    fn parse_empty() {
        assert!(ImageRef::parse("").is_err());
    }

    #[test]
    fn parse_no_slash() {
        assert!(ImageRef::parse("echo:latest").is_err());
    }

    #[test]
    fn parse_whitespace_in_ref() {
        assert!(ImageRef::parse("localhost/ echo").is_err());
    }

    #[test]
    fn parse_blank() {
        assert!(ImageRef::parse(" ").is_err());
    }

    // ── Display ──

    #[test]
    fn display_delegates_to_inner() {
        let r = ImageRef::parse("localhost/echo:latest").unwrap();
        assert_eq!(format!("{}", r), "localhost/echo:latest");
    }

    // ── Serde round-trip ──

    #[test]
    fn json_round_trip() {
        let r = ImageRef::parse("localhost/echo:latest").unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#""localhost/echo:latest""#);
        let back: ImageRef = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn json_rejects_invalid() {
        let result: Result<ImageRef, _> = serde_json::from_str(r#""no-slash""#);
        assert!(result.is_err());
    }

    // ── From / TryFrom ──

    #[test]
    fn into_string() {
        let r = ImageRef::parse("localhost/echo:latest").unwrap();
        let s: String = r.into();
        assert_eq!(s, "localhost/echo:latest");
    }

    #[test]
    fn try_from_string_valid() {
        let r = ImageRef::try_from("localhost/echo:latest".to_string()).unwrap();
        assert_eq!(r.as_str(), "localhost/echo:latest");
    }

    #[test]
    fn try_from_string_invalid() {
        assert!(ImageRef::try_from("bad".to_string()).is_err());
    }
}
