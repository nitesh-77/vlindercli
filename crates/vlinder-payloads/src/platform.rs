//! Platform-level errors — control plane failures, not provider failures.

use serde::{Deserialize, Serialize};

/// A platform error — something the sidecar or queue infrastructure failed on.
///
/// Distinct from provider errors (OpenAI 429, Anthropic overloaded, etc.)
/// which are data plane concerns carried in the per-domain error enums.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformError {
    pub code: PlatformErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlatformErrorCode {
    /// I don't know the error types yet.
    GenericError
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for PlatformError {}
