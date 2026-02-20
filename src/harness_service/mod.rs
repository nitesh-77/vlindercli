//! gRPC Harness Service (ADR 101).
//!
//! Exposes the Harness trait over gRPC for CLI decoupling.
//! - `HarnessServiceServer`: Wraps a CoreHarness, serves gRPC requests
//! - `GrpcHarnessClient`: Implements Harness trait via gRPC calls

mod server;
mod client;

pub use server::HarnessServiceServer;
pub use client::{GrpcHarnessClient, ping_harness};

/// Generated protobuf types.
pub mod proto {
    tonic::include_proto!("vlinder.harness");
}
