//! Content-addressed OCI image digest newtype.
//!
//! Wraps a `sha256:<hex>` string with parse-time validation.
//! Invalid digests are rejected at the boundary — no sentinel values
//! like `"unknown"` can leak into the domain.

use serde::{Deserialize, Serialize};

/// A validated OCI image digest (e.g., `sha256:a80c4f17...`).
///
/// Only `sha256:` is supported — that's the only algorithm Podman produces.
/// The hex portion must be non-empty and contain only `[0-9a-fA-F]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ImageDigest(String);

impl ImageDigest {
    /// Validated constructor.
    ///
    /// Returns `Err` if the input doesn't match `sha256:<hex>`.
    pub fn parse(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();
        let hex = s
            .strip_prefix("sha256:")
            .ok_or_else(|| format!("image digest must start with 'sha256:': {}", s))?;

        if hex.is_empty() {
            return Err(format!("image digest has empty hex portion: {}", s));
        }

        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("image digest contains non-hex characters: {}", s));
        }

        Ok(ImageDigest(s))
    }

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ImageDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ImageDigest> for String {
    fn from(d: ImageDigest) -> Self {
        d.0
    }
}

impl TryFrom<String> for ImageDigest {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        ImageDigest::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Valid digests ──

    #[test]
    fn parse_short_digest() {
        let d = ImageDigest::parse("sha256:abc123").unwrap();
        assert_eq!(d.as_str(), "sha256:abc123");
    }

    #[test]
    fn parse_full_length_digest() {
        // 64-char hex string (typical sha256 output)
        let d = ImageDigest::parse(
            "sha256:a80c4f17a612ca1f5a33cf1ef5e2c3b92e8a1eb3adf09bb133a9e3fbb0a1234",
        )
        .unwrap();
        assert_eq!(
            d.as_str(),
            "sha256:a80c4f17a612ca1f5a33cf1ef5e2c3b92e8a1eb3adf09bb133a9e3fbb0a1234"
        );
    }

    #[test]
    fn parse_uppercase_hex() {
        let d = ImageDigest::parse("sha256:ABC123DEF").unwrap();
        assert_eq!(d.as_str(), "sha256:ABC123DEF");
    }

    // ── Invalid digests ──

    #[test]
    fn parse_empty_string() {
        assert!(ImageDigest::parse("").is_err());
    }

    #[test]
    fn parse_no_prefix() {
        assert!(ImageDigest::parse("abc123").is_err());
    }

    #[test]
    fn parse_empty_hex() {
        assert!(ImageDigest::parse("sha256:").is_err());
    }

    #[test]
    fn parse_non_hex_chars() {
        assert!(ImageDigest::parse("sha256:xyz").is_err());
    }

    #[test]
    fn parse_wrong_algorithm() {
        assert!(ImageDigest::parse("md5:abc").is_err());
    }

    // ── Display ──

    #[test]
    fn display_delegates_to_inner() {
        let d = ImageDigest::parse("sha256:abc123").unwrap();
        assert_eq!(format!("{}", d), "sha256:abc123");
    }

    // ── Serde round-trip ──

    #[test]
    fn json_round_trip() {
        let d = ImageDigest::parse("sha256:abc123").unwrap();
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, r#""sha256:abc123""#);
        let back: ImageDigest = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn json_rejects_invalid() {
        let result: Result<ImageDigest, _> = serde_json::from_str(r#""not-a-digest""#);
        assert!(result.is_err());
    }

    // ── From / TryFrom ──

    #[test]
    fn into_string() {
        let d = ImageDigest::parse("sha256:abc123").unwrap();
        let s: String = d.into();
        assert_eq!(s, "sha256:abc123");
    }

    #[test]
    fn try_from_string_valid() {
        let d = ImageDigest::try_from("sha256:abc123".to_string()).unwrap();
        assert_eq!(d.as_str(), "sha256:abc123");
    }

    #[test]
    fn try_from_string_invalid() {
        assert!(ImageDigest::try_from("bad".to_string()).is_err());
    }
}
