//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - ContainerRuntime: queue-based OCI container agent execution

mod container;
pub(crate) mod http_bridge;
mod http_bridge_server;

pub use container::ContainerRuntime;
pub use container::ImagePolicy;
pub(crate) use container::resolve_image_digest;
