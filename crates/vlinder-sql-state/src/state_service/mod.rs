//! gRPC State Service (ADR 079).
//!
//! Exposes the DagStore trait over gRPC for distributed mode.
//! - `StateServiceServer`: Wraps a DagStore impl, serves gRPC requests
//! - `GrpcStateClient`: Implements DagStore trait via gRPC calls

mod server;
mod client;
mod convert;

pub use server::StateServiceServer;
pub use client::{GrpcStateClient, ping_state_service};

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.state");
}
