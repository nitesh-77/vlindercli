//! Secret store trait definition (ADR 083).
//!
//! Named byte-blob storage for secrets (private keys, NKeys, API keys).
//! The store does not interpret contents — it stores and retrieves.
//!
//! Naming convention:
//! - `agents/{name}/private-key` — Ed25519 private keys (ADR 084)
//! - `nats/{role}/nkey` — NATS NKeys (ADR 085)
//! - `providers/{name}/api-key` — Provider API keys

use std::fmt;

// --- SecretStore Trait ---

/// A store for named secrets (ADR 083).
///
/// Implementations must be thread-safe. Secrets are opaque byte blobs —
/// the store does not interpret, encrypt, or transform the contents.
pub trait SecretStore: Send + Sync {
    /// Store a secret. Overwrites if it already exists.
    fn put(&self, name: &str, value: &[u8]) -> Result<(), SecretStoreError>;

    /// Retrieve a secret by name.
    fn get(&self, name: &str) -> Result<Vec<u8>, SecretStoreError>;

    /// Check whether a secret exists.
    fn exists(&self, name: &str) -> Result<bool, SecretStoreError>;

    /// Delete a secret by name.
    fn delete(&self, name: &str) -> Result<(), SecretStoreError>;
}

// --- Errors ---

#[derive(Debug)]
pub enum SecretStoreError {
    /// Secret does not exist
    NotFound(String),
    /// Failed to store a secret
    StoreFailed(String),
    /// Failed to delete a secret
    DeleteFailed(String),
}

impl fmt::Display for SecretStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretStoreError::NotFound(name) => write!(f, "secret not found: {}", name),
            SecretStoreError::StoreFailed(msg) => write!(f, "store failed: {}", msg),
            SecretStoreError::DeleteFailed(msg) => write!(f, "delete failed: {}", msg),
        }
    }
}

impl std::error::Error for SecretStoreError {}
