//! Ollama native API types for `/api/chat`, `/api/generate`, and `/api/embed`.
//!
//! Minimal types covering required fields + optional metrics fields.
//! Used for route validation (`DeserializeOwned`) and response parsing.

use serde::{Deserialize, Serialize};

// ============================================================================
// /api/chat
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaChatResponse {
    pub model: String,
    pub message: OllamaChatMessage,
    pub done: bool,
    #[serde(default)]
    pub total_duration: Option<u64>,
    #[serde(default)]
    pub prompt_eval_count: Option<u32>,
    #[serde(default)]
    pub eval_count: Option<u32>,
}

// ============================================================================
// /api/generate
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaGenerateRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaGenerateResponse {
    pub model: String,
    pub response: String,
    pub done: bool,
    #[serde(default)]
    pub total_duration: Option<u64>,
    #[serde(default)]
    pub prompt_eval_count: Option<u32>,
    #[serde(default)]
    pub eval_count: Option<u32>,
}

// ============================================================================
// /api/embed
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaEmbedRequest {
    pub model: String,
    pub input: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaEmbedResponse {
    pub embeddings: Vec<Vec<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_round_trip() {
        let req = OllamaChatRequest {
            model: "llama3.2".to_string(),
            messages: vec![OllamaChatMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            stream: Some(false),
            options: None,
        };
        let json = serde_json::to_vec(&req).unwrap();
        let back: OllamaChatRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.model, "llama3.2");
        assert_eq!(back.messages.len(), 1);
    }

    #[test]
    fn chat_response_with_metrics() {
        let json = serde_json::json!({
            "model": "llama3.2",
            "message": {"role": "assistant", "content": "hi"},
            "done": true,
            "total_duration": 500000000,
            "prompt_eval_count": 10,
            "eval_count": 5
        });
        let resp: OllamaChatResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.prompt_eval_count, Some(10));
        assert_eq!(resp.eval_count, Some(5));
    }

    #[test]
    fn chat_response_without_metrics() {
        let json = serde_json::json!({
            "model": "llama3.2",
            "message": {"role": "assistant", "content": "hi"},
            "done": true
        });
        let resp: OllamaChatResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.prompt_eval_count, None);
        assert_eq!(resp.eval_count, None);
    }

    #[test]
    fn generate_request_round_trip() {
        let req = OllamaGenerateRequest {
            model: "llama3.2".to_string(),
            prompt: "Why is the sky blue?".to_string(),
            stream: Some(false),
            options: None,
        };
        let json = serde_json::to_vec(&req).unwrap();
        let back: OllamaGenerateRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.model, "llama3.2");
        assert_eq!(back.prompt, "Why is the sky blue?");
    }

    #[test]
    fn generate_response_with_metrics() {
        let json = serde_json::json!({
            "model": "llama3.2",
            "response": "Because of Rayleigh scattering.",
            "done": true,
            "total_duration": 300000000,
            "prompt_eval_count": 8,
            "eval_count": 12
        });
        let resp: OllamaGenerateResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.prompt_eval_count, Some(8));
        assert_eq!(resp.eval_count, Some(12));
    }

    #[test]
    fn embed_request_round_trip() {
        let req = OllamaEmbedRequest {
            model: "nomic-embed-text".to_string(),
            input: "hello world".to_string(),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let back: OllamaEmbedRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.model, "nomic-embed-text");
        assert_eq!(back.input, "hello world");
    }

    #[test]
    fn embed_response_parses() {
        let json = serde_json::json!({
            "embeddings": [[0.1, 0.2, 0.3]]
        });
        let resp: OllamaEmbedResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.embeddings.len(), 1);
        assert_eq!(resp.embeddings[0], vec![0.1, 0.2, 0.3]);
    }
}
