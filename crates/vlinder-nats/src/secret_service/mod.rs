//! gRPC Secret Store Service.
//!
//! Exposes the SecretStore trait over gRPC for distributed mode.
//! - `SecretServiceServer`: Wraps a SecretStore impl, serves gRPC requests
//! - `GrpcSecretClient`: Implements SecretStore trait via gRPC calls

#[cfg(feature = "client")]
mod client;
#[cfg(feature = "server")]
mod server;

#[cfg(feature = "client")]
pub use client::{ping_secret_service, GrpcSecretClient};
#[cfg(feature = "server")]
pub use server::SecretServiceServer;

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.secret_store");
}
