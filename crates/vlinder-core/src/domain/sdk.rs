//! Agent SDK specification (ADR 074, 075).
//!
//! The complete contract between agents and the platform, expressed at three levels:
//!
//! - `SdkContract` trait: the 11 operations agents can request (kv, vector, infer, embed, delegate)
//! - `AgentAction` enum: agent → platform wire format (POST /handle response body)
//! - `AgentEvent` enum: platform → agent wire format (POST /handle request body)
//!
//! Agent SDKs in any language implement this spec as `ctx.kv_get()`, `ctx.infer()`, etc.
//! The platform fulfills it via `QueueBridge`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// SdkContract — the operations
// ============================================================================

/// The agent SDK contract — what operations agents can request.
///
/// Each method maps to one `AgentAction` variant and one `AgentEvent` variant.
/// The platform fulfills these via `QueueBridge` (or any other implementation).
pub trait SdkContract: Send + Sync {
    // -- Object storage --
    fn kv_get(&self, path: &str) -> Result<Vec<u8>, String>;
    fn kv_put(&self, path: &str, content: &str) -> Result<(), String>;
    fn kv_list(&self, prefix: &str) -> Result<Vec<String>, String>;
    fn kv_delete(&self, path: &str) -> Result<bool, String>;

    // -- Vector storage --
    fn vector_store(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String>;
    fn vector_search(&self, vector: &[f32], limit: u32) -> Result<Vec<VectorMatch>, String>;
    fn vector_delete(&self, key: &str) -> Result<bool, String>;

    // -- Inference --
    fn infer(&self, model: &str, prompt: &str, max_tokens: u32) -> Result<String, String>;

    // -- Embedding --
    fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, String>;

    // -- Delegation (ADR 056) --
    fn delegate(&self, target_agent: &str, input: &str) -> Result<String, String>;
    fn wait(&self, handle: &str) -> Result<Vec<u8>, String>;
}

// ============================================================================
// AgentAction — agent → platform (POST /handle response)
// ============================================================================

/// Agent -> Platform (POST /handle response body).
///
/// Each variant maps 1:1 to an `SdkContract` trait method.
/// The `state` field is opaque agent working memory — the platform
/// round-trips it without interpretation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum AgentAction {
    KvGet { path: String, state: Value },
    KvPut { path: String, content: String, state: Value },
    KvList { prefix: String, state: Value },
    KvDelete { path: String, state: Value },
    VectorStore { key: String, vector: Vec<f32>, metadata: String, state: Value },
    VectorSearch { vector: Vec<f32>, limit: u32, state: Value },
    VectorDelete { key: String, state: Value },
    Infer { model: String, prompt: String, max_tokens: u32, state: Value },
    Embed { model: String, text: String, state: Value },
    Delegate { agent: String, input: String, state: Value },
    Complete { payload: String, state: Value },
}

// ============================================================================
// AgentEvent — platform → agent (POST /handle request)
// ============================================================================

/// Platform -> Agent (POST /handle request body).
///
/// Each variant carries the typed response for the preceding action.
/// `Invoke` starts the conversation; `Error` handles bridge failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    Invoke { input: String, state: Value },
    KvGet { data: Vec<u8>, state: Value },
    KvPut { state: Value },
    KvList { paths: Vec<String>, state: Value },
    KvDelete { existed: bool, state: Value },
    VectorStore { state: Value },
    VectorSearch { matches: Vec<VectorMatch>, state: Value },
    VectorDelete { existed: bool, state: Value },
    Infer { text: String, state: Value },
    Embed { vector: Vec<f32>, state: Value },
    Delegate { output: String, state: Value },
    Error { message: String, state: Value },
}

// ============================================================================
// Shared types
// ============================================================================

/// A single vector search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMatch {
    pub key: String,
    pub metadata: String,
    pub distance: f32,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- AgentAction round-trips --

    #[test]
    fn action_kv_get_round_trip() {
        let action = AgentAction::KvGet {
            path: "/notes.txt".into(),
            state: json!({"step": 1}),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"KvGet"#));
        let parsed: AgentAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, AgentAction::KvGet { ref path, .. } if path == "/notes.txt"));
    }

    #[test]
    fn action_kv_put_round_trip() {
        let action = AgentAction::KvPut {
            path: "/out.txt".into(),
            content: "hello".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"KvPut"#));
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_kv_list_round_trip() {
        let action = AgentAction::KvList {
            prefix: "/docs/".into(),
            state: json!(null),
        };
        let json = serde_json::to_string(&action).unwrap();
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_kv_delete_round_trip() {
        let action = AgentAction::KvDelete {
            path: "/old.txt".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&action).unwrap();
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_vector_store_round_trip() {
        let action = AgentAction::VectorStore {
            key: "doc1".into(),
            vector: vec![0.1, 0.2, 0.3],
            metadata: "test doc".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&action).unwrap();
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_vector_search_round_trip() {
        let action = AgentAction::VectorSearch {
            vector: vec![1.0, 2.0],
            limit: 5,
            state: json!({}),
        };
        let json = serde_json::to_string(&action).unwrap();
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_vector_delete_round_trip() {
        let action = AgentAction::VectorDelete {
            key: "doc1".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&action).unwrap();
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_infer_round_trip() {
        let action = AgentAction::Infer {
            model: "phi3".into(),
            prompt: "What is 2+2?".into(),
            max_tokens: 100,
            state: json!({"history": []}),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"Infer"#));
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_embed_round_trip() {
        let action = AgentAction::Embed {
            model: "nomic".into(),
            text: "hello world".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&action).unwrap();
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_delegate_round_trip() {
        let action = AgentAction::Delegate {
            agent: "summarizer".into(),
            input: "summarize this".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"Delegate"#));
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn action_complete_round_trip() {
        let action = AgentAction::Complete {
            payload: "done".into(),
            state: json!({"final": true}),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"Complete"#));
        let _: AgentAction = serde_json::from_str(&json).unwrap();
    }

    // -- AgentEvent round-trips --

    #[test]
    fn event_invoke_round_trip() {
        let event = AgentEvent::Invoke {
            input: "hello".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"Invoke"#));
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_kv_get_round_trip() {
        let event = AgentEvent::KvGet {
            data: vec![72, 101, 108, 108, 111],
            state: json!({"step": 1}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"KvGet"#));
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_kv_put_round_trip() {
        let event = AgentEvent::KvPut { state: json!({}) };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_kv_list_round_trip() {
        let event = AgentEvent::KvList {
            paths: vec!["/a.txt".into(), "/b.txt".into()],
            state: json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_kv_delete_round_trip() {
        let event = AgentEvent::KvDelete {
            existed: true,
            state: json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_vector_store_round_trip() {
        let event = AgentEvent::VectorStore { state: json!({}) };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_vector_search_round_trip() {
        let event = AgentEvent::VectorSearch {
            matches: vec![VectorMatch {
                key: "doc1".into(),
                metadata: "test".into(),
                distance: 0.5,
            }],
            state: json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_vector_delete_round_trip() {
        let event = AgentEvent::VectorDelete {
            existed: false,
            state: json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_infer_round_trip() {
        let event = AgentEvent::Infer {
            text: "4".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_embed_round_trip() {
        let event = AgentEvent::Embed {
            vector: vec![0.1, 0.2],
            state: json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_delegate_round_trip() {
        let event = AgentEvent::Delegate {
            output: "summary here".into(),
            state: json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_error_round_trip() {
        let event = AgentEvent::Error {
            message: "kv backend not configured".into(),
            state: json!({"step": 2}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"Error"#));
        let _: AgentEvent = serde_json::from_str(&json).unwrap();
    }

    // -- Tag shape verification --

    #[test]
    fn action_tag_is_action() {
        let action = AgentAction::Complete {
            payload: "x".into(),
            state: json!({}),
        };
        let v: Value = serde_json::to_value(&action).unwrap();
        assert_eq!(v["action"], "Complete");
        assert!(v.get("type").is_none());
    }

    #[test]
    fn event_tag_is_type() {
        let event = AgentEvent::Invoke {
            input: "x".into(),
            state: json!({}),
        };
        let v: Value = serde_json::to_value(&event).unwrap();
        assert_eq!(v["type"], "Invoke");
        assert!(v.get("action").is_none());
    }
}
