//! SDK Message — the contract between agents and the platform.
//!
//! Defines all valid operations a WASM agent can request through send().
//! This is the trust boundary: untrusted payloads are validated here.
//! The `op` field determines the variant; remaining fields must match exactly.
//! Unknown ops or missing fields are rejected at deserialization time.

use serde::Deserialize;

use super::storage::{ObjectStorageType, VectorStorageType};

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
}

fn default_max_tokens() -> u32 { 256 }

impl SdkMessage {
    /// Determine (service, backend, operation) for queue routing.
    ///
    /// Returns Err if the agent calls a storage op without declaring the backend.
    pub fn route(
        &self,
        kv_backend: Option<ObjectStorageType>,
        vec_backend: Option<VectorStorageType>,
    ) -> Result<(&'static str, String, &'static str), String> {
        match self {
            SdkMessage::KvGet { .. } => kv_route("get", kv_backend),
            SdkMessage::KvPut { .. } => kv_route("put", kv_backend),
            SdkMessage::KvList { .. } => kv_route("list", kv_backend),
            SdkMessage::KvDelete { .. } => kv_route("delete", kv_backend),
            SdkMessage::VectorStore { .. } => vec_route("store", vec_backend),
            SdkMessage::VectorSearch { .. } => vec_route("search", vec_backend),
            SdkMessage::VectorDelete { .. } => vec_route("delete", vec_backend),
            SdkMessage::Infer { .. } => Ok(("infer", "ollama".to_string(), "run")),
            SdkMessage::Embed { .. } => Ok(("embed", "ollama".to_string(), "run")),
        }
    }
}

fn kv_route(operation: &'static str, backend: Option<ObjectStorageType>) -> Result<(&'static str, String, &'static str), String> {
    let backend = backend.ok_or_else(|| format!("agent called kv-{} but has no object_storage configured", operation))?;
    Ok(("kv", backend.as_str().to_string(), operation))
}

fn vec_route(operation: &'static str, backend: Option<VectorStorageType>) -> Result<(&'static str, String, &'static str), String> {
    let backend = backend.ok_or_else(|| format!("agent called vector-{} but has no vector_storage configured", operation))?;
    Ok(("vec", backend.as_str().to_string(), operation))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: parse JSON into SdkMessage and return its route.
    fn parse_and_route(
        json: &str,
        kv: Option<ObjectStorageType>,
        vec: Option<VectorStorageType>,
    ) -> Result<(&'static str, String, &'static str), String> {
        let msg: SdkMessage = serde_json::from_str(json)
            .map_err(|e| e.to_string())?;
        msg.route(kv, vec)
    }

    #[test]
    fn kv_operations() {
        let (svc, backend, op) = parse_and_route(
            r#"{"op": "kv-get", "path": "/data.txt"}"#,
            Some(ObjectStorageType::Sqlite), None,
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("kv", "sqlite", "get"));

        let (svc, backend, op) = parse_and_route(
            r#"{"op": "kv-put", "path": "/data.txt", "content": "aGVsbG8="}"#,
            Some(ObjectStorageType::InMemory), None,
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("kv", "memory", "put"));
    }

    #[test]
    fn vector_operations() {
        let (svc, backend, op) = parse_and_route(
            r#"{"op": "vector-store", "key": "doc1", "vector": [0.1], "metadata": "test"}"#,
            None, Some(VectorStorageType::SqliteVec),
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("vec", "sqlite-vec", "store"));

        let (svc, backend, op) = parse_and_route(
            r#"{"op": "vector-search", "vector": [0.1], "limit": 5}"#,
            None, Some(VectorStorageType::InMemory),
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("vec", "memory", "search"));
    }

    #[test]
    fn kv_without_backend_fails() {
        let err = parse_and_route(
            r#"{"op": "kv-get", "path": "/data.txt"}"#,
            None, None,
        ).unwrap_err();
        assert!(err.contains("no object_storage configured"));
    }

    #[test]
    fn vector_without_backend_fails() {
        let err = parse_and_route(
            r#"{"op": "vector-store", "key": "k", "vector": [0.1], "metadata": "m"}"#,
            None, None,
        ).unwrap_err();
        assert!(err.contains("no vector_storage configured"));
    }

    #[test]
    fn inference() {
        let (svc, backend, op) = parse_and_route(
            r#"{"op": "infer", "model": "phi3", "prompt": "hello"}"#,
            None, None,
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("infer", "ollama", "run"));

        let (svc, backend, op) = parse_and_route(
            r#"{"op": "embed", "model": "nomic", "text": "hello"}"#,
            None, None,
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("embed", "ollama", "run"));
    }

    #[test]
    fn unknown_op_fails() {
        let err = parse_and_route(
            r#"{"op": "summarize"}"#,
            None, None,
        ).unwrap_err();
        assert!(err.contains("unknown variant"));
    }

    #[test]
    fn missing_fields_fails() {
        let err = parse_and_route(
            r#"{"op": "kv-put", "path": "/data.txt"}"#,
            Some(ObjectStorageType::Sqlite), None,
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
}
