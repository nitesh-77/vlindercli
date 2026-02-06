//! Agent runtime - orchestrates agent execution.
//!
//! Contains:
//! - ContainerRuntime: queue-based OCI container agent execution

mod container;
mod http_bridge;
pub(crate) mod send;

pub use container::ContainerRuntime;
