//! Agent runtime — orchestrates agent execution.
//!
//! Contains:
//! - ContainerRuntime: queue-based OCI container agent execution (via vlinder-podman-runtime)

pub use vlinder_podman_runtime::ContainerRuntime;
pub use vlinder_podman_runtime::PodmanRuntimeConfig;
