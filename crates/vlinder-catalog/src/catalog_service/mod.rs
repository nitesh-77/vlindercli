//! gRPC Catalog Service.
//!
//! Exposes the CatalogService trait over gRPC for distributed mode.
//! - `CatalogServiceServer`: Wraps a CatalogService impl, serves gRPC requests
//! - `GrpcCatalogClient`: Implements CatalogService trait via gRPC calls

mod server;
mod client;
mod convert;

pub use server::CatalogServiceServer;
pub use client::{GrpcCatalogClient, ping_catalog_service};

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.catalog");
}
