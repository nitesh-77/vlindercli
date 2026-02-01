//! Inference Service Handler - LLM inference over queues.
//!
//! Queue:
//! - `infer`: Run inference with a model

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::domain::InferenceEngine;
use crate::queue::{InMemoryQueue, Message, MessageQueue};
use crate::services::inference;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct InferRequest {
    model: String,
    prompt: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    256
}

// ============================================================================
// Handler
// ============================================================================

pub struct InferenceServiceWorker {
    queue: Arc<InMemoryQueue>,
    engines: HashMap<String, Arc<dyn InferenceEngine>>,
}

impl InferenceServiceWorker {
    pub fn new(queue: Arc<InMemoryQueue>) -> Self {
        Self {
            queue,
            engines: HashMap::new(),
        }
    }

    /// Register an inference engine by model name.
    pub fn register(&mut self, model_name: &str, engine: Arc<dyn InferenceEngine>) {
        self.engines.insert(model_name.to_string(), engine);
    }

    /// Process one message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        if let Ok(msg) = self.queue.receive("infer") {
            let response = self.handle_infer(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn send_response(&self, request: &Message, payload: Vec<u8>) {
        let response = Message::response(payload, &request.reply_to, request.id.clone());
        let _ = self.queue.send(&request.reply_to, response);
    }

    fn handle_infer(&self, msg: &Message) -> Vec<u8> {
        let req: InferRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let engine = match self.engines.get(&req.model) {
            Some(e) => e,
            None => return format!("[error] unknown model: {}", req.model).into_bytes(),
        };

        // Call pure service function
        match inference::run_infer(engine.as_ref(), &req.prompt, req.max_tokens) {
            Ok(result) => result.into_bytes(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Message;
    use crate::inference::InMemoryInference;

    #[test]
    fn handles_infer_request() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut handler = InferenceServiceWorker::new(Arc::clone(&queue));

        // Register mock engine
        let engine = Arc::new(InMemoryInference::new("test response"));
        handler.register("test-model", engine);

        // Send infer request
        let payload = serde_json::json!({
            "model": "test-model",
            "prompt": "Hello"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        queue.send("infer", msg).unwrap();

        // Process
        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        assert_eq!(String::from_utf8(response.payload).unwrap(), "test response");
    }

    #[test]
    fn returns_error_for_unknown_model() {
        let queue = Arc::new(InMemoryQueue::new());
        let handler = InferenceServiceWorker::new(Arc::clone(&queue));

        let payload = serde_json::json!({
            "model": "unknown",
            "prompt": "Hello"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        queue.send("infer", msg).unwrap();

        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        let text = String::from_utf8(response.payload).unwrap();
        assert!(text.contains("[error]"));
        assert!(text.contains("unknown"));
    }
}
