//! Service workers - queue-based message handlers.
//!
//! These workers listen on queues and route requests to
//! storage/inference backends.

pub mod dag;

pub use dag::{reconstruct_observable_message, build_dag_node};
