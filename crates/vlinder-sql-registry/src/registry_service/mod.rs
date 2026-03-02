//! gRPC Registry Service.
//!
//! Exposes the Registry trait over gRPC for distributed mode.
//! - `RegistryServer`: Wraps a Registry impl, serves gRPC requests
//! - `GrpcRegistryClient`: Implements Registry trait via gRPC calls

mod client;
mod convert;
mod server;

pub use client::{ping_registry, GrpcRegistryClient};
pub use server::RegistryServiceServer;

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.registry");
}
