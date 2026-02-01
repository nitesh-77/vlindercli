//! Service handlers for queue-based infrastructure operations.
//!
//! These handlers:
//! - Listen on well-known queue names (kv-*, vector-*, infer, embed)
//! - Parse request messages
//! - Call pure service functions from `crate::services`
//! - Send responses to reply queues
//!
//! The handlers are thin adapters that translate queue protocol to
//! service function calls. Business logic lives in `crate::services`.

mod embedding;

pub use embedding::EmbeddingServiceWorker;
