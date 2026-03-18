//! gRPC State Service (ADR 079).
//!
//! Exposes the `DagStore` trait over gRPC for distributed mode.
//! - `StateServiceServer`: Wraps a `DagStore` impl, serves gRPC requests
//! - `GrpcStateClient`: Implements `DagStore` trait via gRPC calls

#[cfg(feature = "client")]
mod client;
mod convert;
#[cfg(feature = "server")]
mod server;

#[cfg(feature = "client")]
pub use client::{ping_state_service, GrpcStateClient};
#[cfg(feature = "server")]
pub use server::StateServiceServer;

/// Generated protobuf types.
pub mod proto {
    #![allow(clippy::doc_markdown, clippy::default_trait_access)]
    tonic::include_proto!("vlinder.state");
}
