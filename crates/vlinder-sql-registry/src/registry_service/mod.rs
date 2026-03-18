//! gRPC Registry Service.
//!
//! Exposes the Registry trait over gRPC for distributed mode.
//! - `RegistryServer`: Wraps a Registry impl, serves gRPC requests
//! - `GrpcRegistryClient`: Implements Registry trait via gRPC calls

#[cfg(feature = "client")]
mod client;
mod convert;
#[cfg(feature = "server")]
mod server;

#[cfg(feature = "client")]
pub use client::{ping_registry, GrpcRegistryClient};
#[cfg(feature = "server")]
pub use server::RegistryServiceServer;

/// Generated protobuf types.
pub mod proto {
    #![allow(clippy::doc_markdown)]
    tonic::include_proto!("vlinder.registry");
}
