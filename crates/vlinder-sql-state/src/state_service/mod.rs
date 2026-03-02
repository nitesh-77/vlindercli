//! gRPC State Service (ADR 079).
//!
//! Exposes the DagStore trait over gRPC for distributed mode.
//! - `StateServiceServer`: Wraps a DagStore impl, serves gRPC requests
//! - `GrpcStateClient`: Implements DagStore trait via gRPC calls

mod client;
mod convert;
mod server;

pub use client::{ping_state_service, GrpcStateClient};
pub use server::StateServiceServer;

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.state");
}
