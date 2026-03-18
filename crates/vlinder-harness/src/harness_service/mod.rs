//! gRPC Harness Service (ADR 101).
//!
//! Exposes the Harness trait over gRPC for CLI decoupling.
//! - `HarnessServiceServer`: Wraps a `CoreHarness`, serves gRPC requests
//! - `GrpcHarnessClient`: Implements `Harness` trait via gRPC calls

#[cfg(feature = "client")]
mod client;
#[cfg(feature = "server")]
mod server;

#[cfg(feature = "client")]
pub use client::{ping_harness, GrpcHarnessClient};
#[cfg(feature = "server")]
pub use server::HarnessServiceServer;

/// Generated protobuf types.
#[allow(clippy::doc_markdown)]
pub mod proto {
    tonic::include_proto!("vlinder.harness");
}
