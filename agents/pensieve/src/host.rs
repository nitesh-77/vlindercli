//! Host function declarations for queue-based Extism PDK
//!
//! These functions communicate with the host runtime via message queues.
//! The runtime handles all routing concerns:
//! - Extracts `op` from payload to determine target queue
//! - Injects `agent_id` for storage isolation
//! - Manages reply queues internally

use extism_pdk::*;
use serde::Serialize;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

// ============================================================================
// Low-level host functions (provided by runtime)
// ============================================================================

#[host_fn]
extern "ExtismHost" {
    /// Send a request and receive the response synchronously.
    /// Payload must include an `op` field for routing.
    /// Runtime injects `agent_id` automatically.
    fn send(payload: Vec<u8>) -> Vec<u8>;
    pub fn get_prompts() -> String;
}

// ============================================================================
// Service call helper
// ============================================================================

fn call_service(op: &str, payload: impl Serialize) -> Result<String, Error> {
    // Build request with op field
    let mut json = serde_json::to_value(&payload)
        .map_err(|e| Error::msg(format!("serialize error: {}", e)))?;

    if let Some(obj) = json.as_object_mut() {
        obj.insert("op".to_string(), serde_json::Value::String(op.to_string()));
    }

    let payload_bytes = serde_json::to_vec(&json)
        .map_err(|e| Error::msg(format!("serialize error: {}", e)))?;

    unsafe {
        let response = send(payload_bytes)?;
        Ok(String::from_utf8_lossy(&response).to_string())
    }
}

// ============================================================================
// High-level wrapper functions
// ============================================================================

pub unsafe fn infer(model: String, prompt: String) -> Result<String, Error> {
    #[derive(Serialize)]
    struct Request { model: String, prompt: String, max_tokens: u32 }
    call_service("infer", Request { model, prompt, max_tokens: 512 })
}

pub unsafe fn embed(model: String, text: String) -> Result<String, Error> {
    #[derive(Serialize)]
    struct Request { model: String, text: String }
    call_service("embed", Request { model, text })
}

pub unsafe fn get_file(path: String) -> Result<Vec<u8>, Error> {
    #[derive(Serialize)]
    struct Request { path: String }
    let response = call_service("kv-get", Request { path })?;
    Ok(response.into_bytes())
}

pub unsafe fn put_file(path: String, content: Vec<u8>) -> Result<String, Error> {
    #[derive(Serialize)]
    struct Request { path: String, content: String }
    call_service("kv-put", Request {
        path,
        content: BASE64.encode(&content),
    })
}

pub unsafe fn list_files(path: String) -> Result<String, Error> {
    #[derive(Serialize)]
    struct Request { path: String }
    call_service("kv-list", Request { path })
}

pub unsafe fn store_embedding(key: String, vector: String, metadata: String) -> Result<String, Error> {
    let vector_array: Vec<f32> = serde_json::from_str(&vector)
        .map_err(|e| Error::msg(format!("parse vector error: {}", e)))?;

    #[derive(Serialize)]
    struct Request { key: String, vector: Vec<f32>, metadata: String }
    call_service("vector-store", Request {
        key,
        vector: vector_array,
        metadata,
    })
}

pub unsafe fn search_by_vector(vector: String, limit: u32) -> Result<String, Error> {
    let vector_array: Vec<f32> = serde_json::from_str(&vector)
        .map_err(|e| Error::msg(format!("parse vector error: {}", e)))?;

    #[derive(Serialize)]
    struct Request { vector: Vec<f32>, limit: u32 }
    call_service("vector-search", Request {
        vector: vector_array,
        limit,
    })
}
