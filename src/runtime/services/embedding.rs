//! Embedding Service Handler - vector embedding over queues.
//!
//! Queue:
//! - `embed`: Generate embeddings for text

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::domain::EmbeddingEngine;
use crate::queue::{InMemoryQueue, Message, MessageQueue};
use crate::services::embedding;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct EmbedRequest {
    model: String,
    text: String,
}

// ============================================================================
// Handler
// ============================================================================

pub struct EmbeddingServiceWorker {
    queue: Arc<InMemoryQueue>,
    engines: HashMap<String, Arc<dyn EmbeddingEngine>>,
}

impl EmbeddingServiceWorker {
    pub fn new(queue: Arc<InMemoryQueue>) -> Self {
        Self {
            queue,
            engines: HashMap::new(),
        }
    }

    /// Register an embedding engine by model name.
    pub fn register(&mut self, model_name: &str, engine: Arc<dyn EmbeddingEngine>) {
        self.engines.insert(model_name.to_string(), engine);
    }

    /// Process one message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        if let Ok(msg) = self.queue.receive("embed") {
            let response = self.handle_embed(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn send_response(&self, request: &Message, payload: Vec<u8>) {
        let response = Message::response(payload, &request.reply_to, request.id.clone());
        let _ = self.queue.send(&request.reply_to, response);
    }

    fn handle_embed(&self, msg: &Message) -> Vec<u8> {
        let req: EmbedRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let engine = match self.engines.get(&req.model) {
            Some(e) => e,
            None => return format!("[error] unknown model: {}", req.model).into_bytes(),
        };

        // Call pure service function
        match embedding::run_embed(engine.as_ref(), &req.text) {
            Ok(vector) => {
                // Serialize to JSON for transport
                match serde_json::to_vec(&vector) {
                    Ok(json) => json,
                    Err(e) => format!("[error] {}", e).into_bytes(),
                }
            }
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Message;
    use crate::embedding::InMemoryEmbedding;

    #[test]
    fn handles_embed_request() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut handler = EmbeddingServiceWorker::new(Arc::clone(&queue));

        // Register mock engine
        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let engine = Arc::new(InMemoryEmbedding::new(canned.clone()));
        handler.register("test-model", engine);

        // Send embed request
        let payload = serde_json::json!({
            "model": "test-model",
            "text": "hello world"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        queue.send("embed", msg).unwrap();

        // Process
        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        let vector: Vec<f32> = serde_json::from_slice(&response.payload).unwrap();
        assert_eq!(vector.len(), 768);
        assert_eq!(vector, canned);
    }

    #[test]
    fn returns_error_for_unknown_model() {
        let queue = Arc::new(InMemoryQueue::new());
        let handler = EmbeddingServiceWorker::new(Arc::clone(&queue));

        let payload = serde_json::json!({
            "model": "unknown",
            "text": "hello"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        queue.send("embed", msg).unwrap();

        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        let text = String::from_utf8(response.payload).unwrap();
        assert!(text.contains("[error]"));
        assert!(text.contains("unknown"));
    }
}
