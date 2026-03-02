//! Lambda runtime — stub for executing agents as AWS Lambda functions.
//!
//! This crate mirrors the structure of `vlinder-podman-runtime` but targets
//! AWS Lambda as the compute backend. Currently a spike: `tick()` is a no-op
//! that logs the number of Lambda-assigned agents.

mod config;
mod runtime;

pub use config::LambdaRuntimeConfig;
pub use runtime::LambdaRuntime;
