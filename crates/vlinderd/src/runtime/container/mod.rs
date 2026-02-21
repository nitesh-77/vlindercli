//! Container runtime module — executes OCI container agents via Podman pods.
//!
//! - `pool`: pod lifecycle management (ContainerRuntime)
//! - `podman`: Podman trait + shared utilities
//! - `podman_api`: PodmanApiClient (primary, REST API)
//! - `podman_cli`: PodmanCliClient (fallback, CLI)

mod podman;
mod podman_api;
mod podman_cli;
mod pool;
mod unix_transport;

pub use pool::ContainerRuntime;
