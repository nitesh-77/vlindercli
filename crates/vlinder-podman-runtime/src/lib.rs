//! Podman container runtime — manages OCI container agents via Podman pods.
//!
//! Extracted from vlinderd to allow alternative runtimes (e.g., Lambda)
//! to implement the same `Runtime` trait.

mod config;
mod podman_client;
mod podman_api;
mod podman_cli;
mod pool;
mod unix_transport;

pub use config::PodmanRuntimeConfig;
pub use pool::ContainerRuntime;
