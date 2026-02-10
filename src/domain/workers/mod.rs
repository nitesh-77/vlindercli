//! Service workers - queue-based message handlers.
//!
//! These workers listen on queues and route requests to
//! storage/inference/embedding backends.

pub mod dag;
mod object;
mod vector;
mod inference;
mod embedding;

pub use dag::DagCaptureWorker;
pub use object::ObjectServiceWorker;
pub use vector::VectorServiceWorker;
pub use inference::InferenceServiceWorker;
pub use embedding::EmbeddingServiceWorker;
