//! gRPC client implementing the ModelCatalog trait.

use std::sync::Mutex;
use tonic::transport::Channel;

use vlinder_core::domain::{CatalogError, Model, ModelCatalog, ModelInfo};
use super::proto::{self, catalog_service_client::CatalogServiceClient};

/// ModelCatalog implementation that makes gRPC calls to a remote Catalog Service.
///
/// Each client is bound to a specific catalog name (e.g., "ollama", "openrouter").
/// The catalog name is sent in every request so the server can dispatch to the
/// correct backend.
pub struct GrpcCatalogClient {
    client: Mutex<CatalogServiceClient<Channel>>,
    runtime: tokio::runtime::Runtime,
    catalog: String,
}

impl GrpcCatalogClient {
    /// Connect to a catalog service server for a specific catalog backend.
    pub fn connect(addr: &str, catalog: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Runtime::new()?;
        let client = runtime.block_on(async {
            CatalogServiceClient::connect(addr.to_string()).await
        })?;

        Ok(Self {
            client: Mutex::new(client),
            runtime,
            catalog: catalog.to_string(),
        })
    }
}

/// Ping a catalog service at the given address, returning its protocol version.
pub fn ping_catalog_service(addr: &str) -> Option<(u32, u32, u32)> {
    let Ok(runtime) = tokio::runtime::Runtime::new() else {
        return None;
    };

    runtime.block_on(async {
        let Ok(mut client) = CatalogServiceClient::connect(addr.to_string()).await else {
            return None;
        };
        client.ping(proto::PingRequest {}).await.ok().map(|r| {
            let v = r.into_inner();
            (v.major, v.minor, v.patch)
        })
    })
}

impl ModelCatalog for GrpcCatalogClient {
    fn resolve(&self, name: &str) -> Result<Model, CatalogError> {
        let request = proto::ResolveRequest {
            catalog: self.catalog.clone(),
            name: name.to_string(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .resolve(request)
                .await
        }).map_err(|e| CatalogError::Network(e.to_string()))?;

        let resp = response.into_inner();
        if let Some(err) = resp.error {
            return Err(CatalogError::Network(err));
        }
        match resp.model {
            Some(m) => Model::try_from(m).map_err(|e| CatalogError::Parse(e)),
            None => Err(CatalogError::NotFound(name.to_string())),
        }
    }

    fn list(&self) -> Result<Vec<ModelInfo>, CatalogError> {
        let request = proto::ListRequest {
            catalog: self.catalog.clone(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .list(request)
                .await
        }).map_err(|e| CatalogError::Network(e.to_string()))?;

        let resp = response.into_inner();
        if let Some(err) = resp.error {
            return Err(CatalogError::Network(err));
        }
        Ok(resp.models.into_iter().map(Into::into).collect())
    }

    fn available(&self, name: &str) -> bool {
        let request = proto::AvailableRequest {
            catalog: self.catalog.clone(),
            name: name.to_string(),
        };

        let result = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .available(request)
                .await
        });

        match result {
            Ok(resp) => resp.into_inner().available,
            Err(_) => false,
        }
    }
}
