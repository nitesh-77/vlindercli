//! gRPC Secret Store Service.
//!
//! Exposes the SecretStore trait over gRPC for distributed mode.
//! - `SecretServiceServer`: Wraps a SecretStore impl, serves gRPC requests
//! - `GrpcSecretClient`: Implements SecretStore trait via gRPC calls

mod server;
mod client;

pub use server::SecretServiceServer;
pub use client::{GrpcSecretClient, ping_secret_service};

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.secret_store");
}
