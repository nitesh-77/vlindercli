//! gRPC client implementing the SecretStore trait.

use std::sync::Mutex;
use tonic::transport::Channel;

use vlinder_core::domain::{SecretStore, SecretStoreError};
use super::proto::{self, secret_store_service_client::SecretStoreServiceClient};

/// SecretStore implementation that makes gRPC calls to a remote Secret Service.
pub struct GrpcSecretClient {
    client: Mutex<SecretStoreServiceClient<Channel>>,
    runtime: tokio::runtime::Runtime,
}

impl GrpcSecretClient {
    /// Connect to a secret service server.
    pub fn connect(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Runtime::new()?;
        let client = runtime.block_on(async {
            SecretStoreServiceClient::connect(addr.to_string()).await
        })?;

        Ok(Self {
            client: Mutex::new(client),
            runtime,
        })
    }
}

/// Ping a secret service at the given address, returning its protocol version.
///
/// Creates a temporary connection and sends a Ping. Returns the server's
/// version on success, None on any connection or transport error.
pub fn ping_secret_service(addr: &str) -> Option<(u32, u32, u32)> {
    let Ok(runtime) = tokio::runtime::Runtime::new() else {
        return None;
    };

    runtime.block_on(async {
        let Ok(mut client) = SecretStoreServiceClient::connect(addr.to_string()).await else {
            return None;
        };
        client.ping(proto::PingRequest {}).await.ok().map(|r| {
            let v = r.into_inner();
            (v.major, v.minor, v.patch)
        })
    })
}

impl SecretStore for GrpcSecretClient {
    fn put(&self, name: &str, value: &[u8]) -> Result<(), SecretStoreError> {
        let request = proto::PutRequest {
            name: name.to_string(),
            value: value.to_vec(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .put(request)
                .await
        }).map_err(|e| SecretStoreError::StoreFailed(e.to_string()))?;

        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(SecretStoreError::StoreFailed(
                resp.error.unwrap_or_else(|| "unknown error".to_string()),
            ))
        }
    }

    fn get(&self, name: &str) -> Result<Vec<u8>, SecretStoreError> {
        let request = proto::GetRequest {
            name: name.to_string(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .get(request)
                .await
        }).map_err(|e| SecretStoreError::StoreFailed(e.to_string()))?;

        let resp = response.into_inner();
        if let Some(err) = resp.error {
            return Err(SecretStoreError::StoreFailed(err));
        }
        if resp.found {
            Ok(resp.value)
        } else {
            Err(SecretStoreError::NotFound(name.to_string()))
        }
    }

    fn exists(&self, name: &str) -> Result<bool, SecretStoreError> {
        let request = proto::ExistsRequest {
            name: name.to_string(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .exists(request)
                .await
        }).map_err(|e| SecretStoreError::StoreFailed(e.to_string()))?;

        let resp = response.into_inner();
        if let Some(err) = resp.error {
            return Err(SecretStoreError::StoreFailed(err));
        }
        Ok(resp.exists)
    }

    fn delete(&self, name: &str) -> Result<(), SecretStoreError> {
        let request = proto::DeleteRequest {
            name: name.to_string(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .delete(request)
                .await
        }).map_err(|e| SecretStoreError::DeleteFailed(e.to_string()))?;

        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(SecretStoreError::DeleteFailed(
                resp.error.unwrap_or_else(|| "unknown error".to_string()),
            ))
        }
    }
}
