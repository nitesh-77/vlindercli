//! Service workers - queue-based message handlers.
//!
//! These workers listen on queues and route requests to
//! storage/inference backends.

pub mod dag;
mod object;
mod vector;

pub use dag::{reconstruct_observable_message, build_dag_node};
pub use object::{ObjectServiceWorker, OpenObjectStorage, OpenStateStore};
pub use vector::{VectorServiceWorker, OpenVectorStorage};
