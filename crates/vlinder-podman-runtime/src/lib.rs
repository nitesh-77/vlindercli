//! Podman container runtime — manages OCI container agents via Podman pods.
//!
//! Extracted from vlinderd to allow alternative runtimes (e.g., Lambda)
//! to implement the same `Runtime` trait.

mod podman;
mod podman_api;
mod podman_cli;
mod pool;
mod unix_transport;

pub use pool::ContainerRuntime;
pub use pool::PodmanRuntimeConfig;
