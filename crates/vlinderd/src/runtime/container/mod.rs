//! Container runtime module — executes OCI container agents via Podman pods.
//!
//! - `runtime`: tick-loop orchestrator (ContainerRuntime)
//! - `pool`: pod lifecycle management (ContainerPool)
//! - `podman`: Podman trait + shared utilities
//! - `podman_api`: PodmanApiClient (primary, REST API)
//! - `podman_cli`: PodmanCliClient (fallback, CLI)

mod podman;
mod podman_api;
mod podman_cli;
mod pool;
mod runtime;
mod unix_transport;

pub use runtime::ContainerRuntime;
