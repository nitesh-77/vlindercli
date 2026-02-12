//! SDK Message — the contract between agents and the platform.
//!
//! Defines all valid operations a WASM agent can request through send().
//! This is the trust boundary: untrusted payloads are validated here.
//! The `op` field determines the variant; remaining fields must match exactly.
//! Unknown ops or missing fields are rejected at deserialization time.

use std::collections::HashMap;

use serde::Deserialize;

use super::storage::{ObjectStorageType, VectorStorageType};

/// A single hop in message routing.
///
/// Determines the next queue subject a message is sent to.
#[derive(Debug)]
pub struct Hop {
    pub service: &'static str,
    pub backend: String,
    pub operation: &'static str,
}

/// All valid operations an agent can request.
///
/// Fields are validated by serde but not read directly — the raw payload
/// bytes are forwarded to service workers for their own deserialization.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
#[allow(dead_code)]
pub enum SdkMessage {
    // -- Object storage --
    KvGet { path: String },
    KvPut { path: String, content: String },
    KvList { path: String },
    KvDelete { path: String },
    // -- Vector storage --
    VectorStore { key: String, vector: Vec<f32>, metadata: String },
    VectorSearch { vector: Vec<f32>, limit: u32 },
    VectorDelete { key: String },
    // -- Inference --
    Infer { model: String, prompt: String, #[serde(default = "default_max_tokens")] max_tokens: u32 },
    // -- Embedding --
    Embed { model: String, text: String },
    // -- Delegation (ADR 056) --
    Delegate { agent: String, input: String },
    Wait { handle: String },
}

fn default_max_tokens() -> u32 { 256 }

impl SdkMessage {
    /// Determine the next hop for this message.
    ///
    /// Returns Err if the agent calls a storage op without declaring the backend,
    /// or if the agent calls infer/embed with a model not declared in its requirements.
    pub fn hop(
        &self,
        kv_backend: Option<ObjectStorageType>,
        vec_backend: Option<VectorStorageType>,
        model_backends: &HashMap<String, String>,
    ) -> Result<Hop, String> {
        match self {
            SdkMessage::KvGet { .. } => kv_hop("get", kv_backend),
            SdkMessage::KvPut { .. } => kv_hop("put", kv_backend),
            SdkMessage::KvList { .. } => kv_hop("list", kv_backend),
            SdkMessage::KvDelete { .. } => kv_hop("delete", kv_backend),
            SdkMessage::VectorStore { .. } => vec_hop("store", vec_backend),
            SdkMessage::VectorSearch { .. } => vec_hop("search", vec_backend),
            SdkMessage::VectorDelete { .. } => vec_hop("delete", vec_backend),
            SdkMessage::Infer { model, .. } => infer_hop(model, model_backends),
            SdkMessage::Embed { model, .. } => embed_hop(model, model_backends),
            SdkMessage::Delegate { .. } => Err("delegate is handled by QueueBridge, not hop routing".to_string()),
            SdkMessage::Wait { .. } => Err("wait is handled by QueueBridge, not hop routing".to_string()),
        }
    }
}

fn kv_hop(operation: &'static str, backend: Option<ObjectStorageType>) -> Result<Hop, String> {
    let backend = backend.ok_or_else(|| format!("agent called kv-{} but has no object_storage configured", operation))?;
    Ok(Hop { service: "kv", backend: backend.as_str().to_string(), operation })
}

fn vec_hop(operation: &'static str, backend: Option<VectorStorageType>) -> Result<Hop, String> {
    let backend = backend.ok_or_else(|| format!("agent called vector-{} but has no vector_storage configured", operation))?;
    Ok(Hop { service: "vec", backend: backend.as_str().to_string(), operation })
}

fn infer_hop(model: &str, model_backends: &HashMap<String, String>) -> Result<Hop, String> {
    let backend = model_backends.get(model)
        .ok_or_else(|| format!("agent called infer with undeclared model '{}'", model))?;
    Ok(Hop { service: "infer", backend: backend.clone(), operation: "run" })
}

fn embed_hop(model: &str, model_backends: &HashMap<String, String>) -> Result<Hop, String> {
    let backend = model_backends.get(model)
        .ok_or_else(|| format!("agent called embed with undeclared model '{}'", model))?;
    Ok(Hop { service: "embed", backend: backend.clone(), operation: "run" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_models() -> HashMap<String, String> {
        HashMap::new()
    }

    fn models_with(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// Helper: parse JSON into SdkMessage and return its hop.
    fn parse_and_hop(
        json: &str,
        kv: Option<ObjectStorageType>,
        vec: Option<VectorStorageType>,
        model_backends: &HashMap<String, String>,
    ) -> Result<Hop, String> {
        let msg: SdkMessage = serde_json::from_str(json)
            .map_err(|e| e.to_string())?;
        msg.hop(kv, vec, model_backends)
    }

    #[test]
    fn kv_operations() {
        let r = parse_and_hop(
            r#"{"op": "kv-get", "path": "/data.txt"}"#,
            Some(ObjectStorageType::Sqlite), None, &empty_models(),
        ).unwrap();
        assert_eq!((r.service, r.backend.as_str(), r.operation), ("kv", "sqlite", "get"));

        let r = parse_and_hop(
            r#"{"op": "kv-put", "path": "/data.txt", "content": "aGVsbG8="}"#,
            Some(ObjectStorageType::InMemory), None, &empty_models(),
        ).unwrap();
        assert_eq!((r.service, r.backend.as_str(), r.operation), ("kv", "memory", "put"));
    }

    #[test]
    fn vector_operations() {
        let r = parse_and_hop(
            r#"{"op": "vector-store", "key": "doc1", "vector": [0.1], "metadata": "test"}"#,
            None, Some(VectorStorageType::SqliteVec), &empty_models(),
        ).unwrap();
        assert_eq!((r.service, r.backend.as_str(), r.operation), ("vec", "sqlite-vec", "store"));

        let r = parse_and_hop(
            r#"{"op": "vector-search", "vector": [0.1], "limit": 5}"#,
            None, Some(VectorStorageType::InMemory), &empty_models(),
        ).unwrap();
        assert_eq!((r.service, r.backend.as_str(), r.operation), ("vec", "memory", "search"));
    }

    #[test]
    fn kv_without_backend_fails() {
        let err = parse_and_hop(
            r#"{"op": "kv-get", "path": "/data.txt"}"#,
            None, None, &empty_models(),
        ).unwrap_err();
        assert!(err.contains("no object_storage configured"));
    }

    #[test]
    fn vector_without_backend_fails() {
        let err = parse_and_hop(
            r#"{"op": "vector-store", "key": "k", "vector": [0.1], "metadata": "m"}"#,
            None, None, &empty_models(),
        ).unwrap_err();
        assert!(err.contains("no vector_storage configured"));
    }

    #[test]
    fn inference_routes_to_declared_backend() {
        let models = models_with(&[("phi3", "ollama"), ("nomic", "ollama")]);
        let r = parse_and_hop(
            r#"{"op": "infer", "model": "phi3", "prompt": "hello"}"#,
            None, None, &models,
        ).unwrap();
        assert_eq!((r.service, r.backend.as_str(), r.operation), ("infer", "ollama", "run"));

        let r = parse_and_hop(
            r#"{"op": "embed", "model": "nomic", "text": "hello"}"#,
            None, None, &models,
        ).unwrap();
        assert_eq!((r.service, r.backend.as_str(), r.operation), ("embed", "ollama", "run"));
    }

    #[test]
    fn inference_routes_openrouter_model() {
        let models = models_with(&[
            ("phi3", "ollama"),
            ("claude-sonnet", "openrouter"),
        ]);

        let r = parse_and_hop(
            r#"{"op": "infer", "model": "phi3", "prompt": "hello"}"#,
            None, None, &models,
        ).unwrap();
        assert_eq!((r.service, r.backend.as_str(), r.operation), ("infer", "ollama", "run"));

        let r = parse_and_hop(
            r#"{"op": "infer", "model": "claude-sonnet", "prompt": "hello"}"#,
            None, None, &models,
        ).unwrap();
        assert_eq!((r.service, r.backend.as_str(), r.operation), ("infer", "openrouter", "run"));
    }

    #[test]
    fn infer_with_undeclared_model_fails() {
        let models = models_with(&[("phi3", "ollama")]);
        let err = parse_and_hop(
            r#"{"op": "infer", "model": "unknown-model", "prompt": "hello"}"#,
            None, None, &models,
        ).unwrap_err();
        assert!(err.contains("undeclared model"));
    }

    #[test]
    fn embed_with_undeclared_model_fails() {
        let err = parse_and_hop(
            r#"{"op": "embed", "model": "nomic", "text": "hello"}"#,
            None, None, &empty_models(),
        ).unwrap_err();
        assert!(err.contains("undeclared model"));
    }

    #[test]
    fn unknown_op_fails() {
        let err = parse_and_hop(
            r#"{"op": "summarize"}"#,
            None, None, &empty_models(),
        ).unwrap_err();
        assert!(err.contains("unknown variant"));
    }

    #[test]
    fn missing_fields_fails() {
        let err = parse_and_hop(
            r#"{"op": "kv-put", "path": "/data.txt"}"#,
            Some(ObjectStorageType::Sqlite), None, &empty_models(),
        ).unwrap_err();
        assert!(err.contains("content"));
    }

    #[test]
    fn infer_defaults_max_tokens() {
        let msg: SdkMessage = serde_json::from_str(
            r#"{"op": "infer", "model": "phi3", "prompt": "hello"}"#,
        ).unwrap();
        match msg {
            SdkMessage::Infer { max_tokens, .. } => assert_eq!(max_tokens, 256),
            _ => panic!("expected Infer variant"),
        }
    }

    // --- Delegation tests (ADR 056) ---

    #[test]
    fn delegate_parses() {
        let msg: SdkMessage = serde_json::from_str(
            r#"{"op": "delegate", "agent": "summarizer", "input": "summarize this"}"#,
        ).unwrap();
        match msg {
            SdkMessage::Delegate { agent, input } => {
                assert_eq!(agent, "summarizer");
                assert_eq!(input, "summarize this");
            }
            _ => panic!("expected Delegate variant"),
        }
    }

    #[test]
    fn wait_parses() {
        let msg: SdkMessage = serde_json::from_str(
            r#"{"op": "wait", "handle": "vlinder.sub.reply.abc123"}"#,
        ).unwrap();
        match msg {
            SdkMessage::Wait { handle } => {
                assert_eq!(handle, "vlinder.sub.reply.abc123");
            }
            _ => panic!("expected Wait variant"),
        }
    }

    #[test]
    fn delegate_hop_returns_err() {
        let err = parse_and_hop(
            r#"{"op": "delegate", "agent": "summarizer", "input": "test"}"#,
            None, None, &empty_models(),
        ).unwrap_err();
        assert!(err.contains("delegate"));
    }

    #[test]
    fn wait_hop_returns_err() {
        let err = parse_and_hop(
            r#"{"op": "wait", "handle": "reply.subject"}"#,
            None, None, &empty_models(),
        ).unwrap_err();
        assert!(err.contains("wait"));
    }
}
