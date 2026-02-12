//! Container runtime module — executes OCI container agents.
//!
//! - `runtime`: tick-loop orchestrator (ContainerRuntime)
//! - `pool`: container lifecycle management (ContainerPool)
//! - `podman`: Podman CLI abstraction
//! - `dispatch`: HTTP dispatch to containers + in-flight task tracking

mod dispatch;
mod podman;
mod podman_socket;
mod pool;
mod runtime;
mod unix_transport;

pub use pool::ImagePolicy;
pub use runtime::ContainerRuntime;
pub(crate) use podman::resolve_image_digest;
