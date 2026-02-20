//! Container runtime module — executes OCI container agents.
//!
//! - `runtime`: tick-loop orchestrator (ContainerRuntime)
//! - `pool`: container lifecycle management (ContainerPool)
//! - `podman`: Podman trait + shared utilities
//! - `podman_api`: PodmanApiClient (primary, REST API)
//! - `podman_cli`: PodmanCliClient (fallback, CLI)
//! - `dispatch`: HTTP dispatch to containers + in-flight task tracking

mod dispatch;
mod podman;
mod podman_api;
mod podman_cli;
mod pool;
mod runtime;
mod unix_transport;

pub use pool::ImagePolicy;
pub use runtime::ContainerRuntime;
