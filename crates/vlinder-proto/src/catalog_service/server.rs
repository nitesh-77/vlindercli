//! gRPC server wrapping ModelCatalog implementations.

use std::collections::HashMap;
use std::sync::Arc;
use tonic::{Request, Response, Status};

use vlinder_core::domain::ModelCatalog;
use super::proto::{
    catalog_service_server::CatalogService,
    PingRequest, SemVer,
    ResolveRequest, ResolveResponse,
    ListRequest, ListResponse,
    AvailableRequest, AvailableResponse,
};

/// gRPC server that dispatches catalog requests to the appropriate backend.
///
/// Holds a map of catalog name → implementation. The request's `catalog` field
/// selects which backend handles the call.
pub struct CatalogServiceServer {
    catalogs: HashMap<String, Arc<dyn ModelCatalog>>,
}

impl CatalogServiceServer {
    pub fn new(catalogs: HashMap<String, Arc<dyn ModelCatalog>>) -> Self {
        Self { catalogs }
    }

    /// Create a tonic service from this server.
    pub fn into_service(self) -> super::proto::catalog_service_server::CatalogServiceServer<Self> {
        super::proto::catalog_service_server::CatalogServiceServer::new(self)
    }

    fn get_catalog(&self, name: &str) -> Result<Arc<dyn ModelCatalog>, Status> {
        self.catalogs
            .get(name)
            .cloned()
            .ok_or_else(|| Status::not_found(format!("unknown catalog: {}", name)))
    }
}

#[tonic::async_trait]
impl CatalogService for CatalogServiceServer {
    async fn ping(
        &self,
        _request: Request<PingRequest>,
    ) -> Result<Response<SemVer>, Status> {
        Ok(Response::new(SemVer {
            major: 0,
            minor: 0,
            patch: 1,
        }))
    }

    async fn resolve(
        &self,
        request: Request<ResolveRequest>,
    ) -> Result<Response<ResolveResponse>, Status> {
        let req = request.into_inner();
        let catalog = self.get_catalog(&req.catalog)?;
        let name = req.name;

        let result = tokio::task::spawn_blocking(move || {
            catalog.resolve(&name)
        }).await.map_err(|e| Status::internal(e.to_string()))?;

        match result {
            Ok(model) => Ok(Response::new(ResolveResponse {
                model: Some(model.into()),
                error: None,
            })),
            Err(e) => Ok(Response::new(ResolveResponse {
                model: None,
                error: Some(e.to_string()),
            })),
        }
    }

    async fn list(
        &self,
        request: Request<ListRequest>,
    ) -> Result<Response<ListResponse>, Status> {
        let req = request.into_inner();
        let catalog = self.get_catalog(&req.catalog)?;

        let result = tokio::task::spawn_blocking(move || {
            catalog.list()
        }).await.map_err(|e| Status::internal(e.to_string()))?;

        match result {
            Ok(models) => Ok(Response::new(ListResponse {
                models: models.into_iter().map(Into::into).collect(),
                error: None,
            })),
            Err(e) => Ok(Response::new(ListResponse {
                models: Vec::new(),
                error: Some(e.to_string()),
            })),
        }
    }

    async fn available(
        &self,
        request: Request<AvailableRequest>,
    ) -> Result<Response<AvailableResponse>, Status> {
        let req = request.into_inner();
        let catalog = self.get_catalog(&req.catalog)?;
        let name = req.name;

        let result = tokio::task::spawn_blocking(move || {
            catalog.available(&name)
        }).await.map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(AvailableResponse {
            available: result,
            error: None,
        }))
    }
}
