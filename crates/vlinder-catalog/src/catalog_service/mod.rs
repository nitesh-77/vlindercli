//! gRPC Catalog Service.
//!
//! Exposes the CatalogService trait over gRPC for distributed mode.
//! - `CatalogServiceServer`: Wraps a CatalogService impl, serves gRPC requests
//! - `GrpcCatalogClient`: Implements CatalogService trait via gRPC calls

mod client;
mod convert;
mod server;

pub use client::{ping_catalog_service, GrpcCatalogClient};
pub use server::CatalogServiceServer;

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.catalog");
}
