//! Service workers - queue-based message handlers.
//!
//! These workers listen on queues and route requests to
//! storage/inference/embedding backends.

mod object;

pub use object::ObjectServiceWorker;
