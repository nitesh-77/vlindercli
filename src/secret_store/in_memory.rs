//! In-memory secret store implementation.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::domain::{SecretStore, SecretStoreError};

/// In-memory secret store for single-process use and testing.
///
/// Secrets are stored in a `HashMap` behind a `Mutex` — same pattern
/// as `InMemoryQueue`. No persistence, no encryption.
pub struct InMemorySecretStore {
    secrets: Mutex<HashMap<String, Vec<u8>>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self {
            secrets: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemorySecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for InMemorySecretStore {
    fn put(&self, name: &str, value: &[u8]) -> Result<(), SecretStoreError> {
        let mut secrets = self.secrets.lock().unwrap();
        secrets.insert(name.to_string(), value.to_vec());
        Ok(())
    }

    fn get(&self, name: &str) -> Result<Vec<u8>, SecretStoreError> {
        let secrets = self.secrets.lock().unwrap();
        secrets
            .get(name)
            .cloned()
            .ok_or_else(|| SecretStoreError::NotFound(name.to_string()))
    }

    fn exists(&self, name: &str) -> Result<bool, SecretStoreError> {
        let secrets = self.secrets.lock().unwrap();
        Ok(secrets.contains_key(name))
    }

    fn delete(&self, name: &str) -> Result<(), SecretStoreError> {
        let mut secrets = self.secrets.lock().unwrap();
        if secrets.remove(name).is_some() {
            Ok(())
        } else {
            Err(SecretStoreError::DeleteFailed(format!(
                "secret not found: {}",
                name
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_returns_value() {
        let store = InMemorySecretStore::new();
        store.put("agents/echo/private-key", b"secret-bytes").unwrap();

        let value = store.get("agents/echo/private-key").unwrap();
        assert_eq!(value, b"secret-bytes");
    }

    #[test]
    fn get_missing_key_returns_not_found() {
        let store = InMemorySecretStore::new();

        let result = store.get("agents/missing/private-key");
        assert!(matches!(result, Err(SecretStoreError::NotFound(_))));
    }

    #[test]
    fn exists_returns_true_after_put_false_before() {
        let store = InMemorySecretStore::new();

        assert!(!store.exists("agents/echo/private-key").unwrap());

        store.put("agents/echo/private-key", b"key").unwrap();

        assert!(store.exists("agents/echo/private-key").unwrap());
    }

    #[test]
    fn delete_removes_secret() {
        let store = InMemorySecretStore::new();
        store.put("agents/echo/private-key", b"key").unwrap();

        store.delete("agents/echo/private-key").unwrap();

        assert!(!store.exists("agents/echo/private-key").unwrap());
        assert!(matches!(
            store.get("agents/echo/private-key"),
            Err(SecretStoreError::NotFound(_))
        ));
    }

    #[test]
    fn delete_missing_key_returns_delete_failed() {
        let store = InMemorySecretStore::new();

        let result = store.delete("agents/missing/private-key");
        assert!(matches!(result, Err(SecretStoreError::DeleteFailed(_))));
    }

    #[test]
    fn put_overwrites_existing_value() {
        let store = InMemorySecretStore::new();
        store.put("agents/echo/private-key", b"old-key").unwrap();
        store.put("agents/echo/private-key", b"new-key").unwrap();

        let value = store.get("agents/echo/private-key").unwrap();
        assert_eq!(value, b"new-key");
    }
}
