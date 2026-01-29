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

mod object;
mod vector;
mod inference;
mod embedding;

pub use object::ObjectServiceHandler;
pub use vector::VectorServiceHandler;
pub use inference::InferenceServiceHandler;
pub use embedding::EmbeddingServiceHandler;
