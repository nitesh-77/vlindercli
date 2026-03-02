//! gRPC Harness Service (ADR 101).
//!
//! Exposes the Harness trait over gRPC for CLI decoupling.
//! - `HarnessServiceServer`: Wraps a CoreHarness, serves gRPC requests
//! - `GrpcHarnessClient`: Implements Harness trait via gRPC calls

mod client;
mod server;

pub use client::{ping_harness, GrpcHarnessClient};
pub use server::HarnessServiceServer;

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.harness");
}
