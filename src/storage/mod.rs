//! MIT-only storage for agents
//!
//! Uses rusqlite (bundled) + sqlite-vec for file and vector storage.
//! Single .db file per agent containing:
//! - files table for virtual filesystem (ObjectStorage)
//! - vec_items virtual table for embeddings (VectorStorage)

mod object;
mod vector;

pub use object::{ObjectStorage, SqliteObjectStorage, InMemoryObjectStorage, open_object_storage};
pub use vector::{VectorStorage, SqliteVectorStorage, InMemoryVectorStorage, open_vector_storage};
