//! Agent runtime — orchestrates agent execution.
//!
//! Contains:
//! - ContainerRuntime: queue-based OCI container agent execution

mod container;

pub use container::ContainerRuntime;
pub use container::ImagePolicy;
