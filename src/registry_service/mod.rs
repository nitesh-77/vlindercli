//! gRPC Registry Service.
//!
//! Exposes the Registry trait over gRPC for distributed mode.
//! - `RegistryServer`: Wraps a Registry impl, serves gRPC requests
//! - `GrpcRegistryClient`: Implements Registry trait via gRPC calls

mod server;
mod client;
mod convert;

pub use server::RegistryServiceServer;
pub use client::{GrpcRegistryClient, ping_registry};

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.registry");
}
