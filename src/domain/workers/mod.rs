//! Service workers - queue-based message handlers.
//!
//! These workers listen on queues and route requests to
//! storage/inference/embedding backends.

mod object;
mod vector;
mod inference;
mod embedding;

pub use object::ObjectServiceWorker;
pub use vector::VectorServiceWorker;
pub use inference::InferenceServiceWorker;
pub use embedding::EmbeddingServiceWorker;
