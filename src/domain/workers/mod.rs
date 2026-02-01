//! Service workers - queue-based message handlers.
//!
//! These workers listen on queues and route requests to
//! storage/inference/embedding backends.

mod object;
mod vector;

pub use object::ObjectServiceWorker;
pub use vector::VectorServiceWorker;
