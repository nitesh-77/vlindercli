//! Service workers - queue-based message handlers.
//!
//! These workers listen on queues and route requests to
//! storage/inference/embedding backends.

pub mod dag;
mod object;
mod vector;
mod embedding;

pub use dag::{reconstruct_observable_message, build_dag_node};
pub use object::{ObjectServiceWorker, OpenObjectStorage, OpenStateStore};
pub use vector::{VectorServiceWorker, OpenVectorStorage};
pub use embedding::{EmbeddingServiceWorker, OpenEmbeddingEngine};
