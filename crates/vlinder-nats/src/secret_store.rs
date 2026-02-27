//! NATS KV-backed secret store (ADR 083).
//!
//! Provides a sync facade over the async NATS KV client. The tokio runtime
//! is owned internally, so callers use simple blocking APIs.

use std::sync::Arc;

use async_nats::jetstream::{self, kv};
use tokio::runtime::Runtime;

use vlinder_core::domain::{SecretStore, SecretStoreError};

/// NATS KV secret store.
///
/// Sync facade over async internals. Clone is cheap (Arc).
#[derive(Clone)]
pub struct NatsSecretStore {
    inner: Arc<NatsSecretStoreInner>,
}

struct NatsSecretStoreInner {
    runtime: Runtime,
    kv: kv::Store,
}

impl NatsSecretStore {
    /// Connect to a NATS server and create/open the `vlinder-secrets` KV bucket.
    pub fn connect(url: &str) -> Result<Self, SecretStoreError> {
        let runtime = Runtime::new()
            .map_err(|e| SecretStoreError::StoreFailed(format!("failed to create runtime: {}", e)))?;

        let kv = runtime.block_on(async {
            let client = async_nats::connect(url)
                .await
                .map_err(|e| SecretStoreError::StoreFailed(format!("failed to connect: {}", e)))?;

            let jetstream = jetstream::new(client);

            let kv = jetstream
                .create_key_value(kv::Config {
                    bucket: "vlinder-secrets".to_string(),
                    history: 1,
                    ..Default::default()
                })
                .await
                .map_err(|e| SecretStoreError::StoreFailed(format!("failed to create KV bucket: {}", e)))?;

            Ok::<_, SecretStoreError>(kv)
        })?;

        Ok(Self {
            inner: Arc::new(NatsSecretStoreInner { runtime, kv }),
        })
    }
}

impl SecretStore for NatsSecretStore {
    fn put(&self, name: &str, value: &[u8]) -> Result<(), SecretStoreError> {
        let value = value.to_vec();
        self.inner.runtime.block_on(async {
            self.inner
                .kv
                .put(name, value.into())
                .await
                .map_err(|e| SecretStoreError::StoreFailed(e.to_string()))?;
            Ok(())
        })
    }

    fn get(&self, name: &str) -> Result<Vec<u8>, SecretStoreError> {
        self.inner.runtime.block_on(async {
            self.inner
                .kv
                .get(name)
                .await
                .map_err(|e| SecretStoreError::StoreFailed(e.to_string()))?
                .map(|bytes| bytes.to_vec())
                .ok_or_else(|| SecretStoreError::NotFound(name.to_string()))
        })
    }

    fn exists(&self, name: &str) -> Result<bool, SecretStoreError> {
        self.inner.runtime.block_on(async {
            let result = self.inner
                .kv
                .get(name)
                .await
                .map_err(|e| SecretStoreError::StoreFailed(e.to_string()))?;
            Ok(result.is_some())
        })
    }

    fn delete(&self, name: &str) -> Result<(), SecretStoreError> {
        self.inner.runtime.block_on(async {
            self.inner
                .kv
                .delete(name)
                .await
                .map_err(|e| SecretStoreError::DeleteFailed(e.to_string()))
        })
    }
}
