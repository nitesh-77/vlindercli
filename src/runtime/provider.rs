//! Provider - aggregates service workers for the runtime.
//!
//! The Provider holds all infrastructure service workers (object storage,
//! vector storage, inference, embedding) and processes their queues.

use std::sync::Arc;

use crate::queue::InMemoryQueue;

use super::services::{
    EmbeddingServiceWorker, InferenceServiceWorker,
    ObjectServiceWorker, VectorServiceWorker,
};

/// Shared service provider that can be accessed from host functions.
///
/// Aggregates all service workers and provides a unified tick interface
/// for processing service messages.
pub struct Provider {
    pub object: ObjectServiceWorker,
    pub vector: VectorServiceWorker,
    pub inference: InferenceServiceWorker,
    pub embedding: EmbeddingServiceWorker,
}

impl Provider {
    /// Create a new Provider with all service workers.
    pub fn new(queue: Arc<InMemoryQueue>) -> Self {
        Self {
            object: ObjectServiceWorker::new(Arc::clone(&queue)),
            vector: VectorServiceWorker::new(Arc::clone(&queue)),
            inference: InferenceServiceWorker::new(Arc::clone(&queue)),
            embedding: EmbeddingServiceWorker::new(Arc::clone(&queue)),
        }
    }

    /// Process one service message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        if self.object.tick() { return true; }
        if self.vector.tick() { return true; }
        if self.inference.tick() { return true; }
        if self.embedding.tick() { return true; }
        false
    }
}
