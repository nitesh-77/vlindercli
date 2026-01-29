//! Host function declarations for queue-based Extism PDK
//!
//! These functions communicate with the host runtime via message queues.
//! The pattern: send request to service queue, receive response from reply queue.

use extism_pdk::*;
use serde::{Deserialize, Serialize};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

const NAMESPACE: &str = "pensieve";

// ============================================================================
// Low-level host functions (provided by runtime)
// ============================================================================

#[host_fn]
extern "ExtismHost" {
    fn send(queue_name: String, payload: Vec<u8>, reply_to: String) -> String;
    fn receive(queue_name: String) -> String;
    pub fn get_prompts() -> String;
}

// ============================================================================
// Queue communication helpers
// ============================================================================

#[derive(Deserialize)]
struct QueueMessage {
    payload: String,
}

fn call_service(queue: &str, payload: impl Serialize) -> Result<String, Error> {
    let reply_queue = format!("pensieve-reply-{}", uuid());
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| Error::msg(format!("serialize error: {}", e)))?;

    unsafe {
        let _msg_id = send(queue.to_string(), payload_bytes, reply_queue.clone())?;
        let response_json = receive(reply_queue)?;
        let msg: QueueMessage = serde_json::from_str(&response_json)
            .map_err(|e| Error::msg(format!("parse response error: {}", e)))?;
        Ok(msg.payload)
    }
}

fn uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
}

// ============================================================================
// High-level wrapper functions (same signatures as before)
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
    struct Request { namespace: String, path: String }
    let response = call_service("kv-get", Request {
        namespace: NAMESPACE.to_string(),
        path,
    })?;
    Ok(response.into_bytes())
}

pub unsafe fn put_file(path: String, content: Vec<u8>) -> Result<String, Error> {
    #[derive(Serialize)]
    struct Request { namespace: String, path: String, content: String }
    call_service("kv-put", Request {
        namespace: NAMESPACE.to_string(),
        path,
        content: BASE64.encode(&content),
    })
}

pub unsafe fn list_files(path: String) -> Result<String, Error> {
    #[derive(Serialize)]
    struct Request { namespace: String, path: String }
    call_service("kv-list", Request {
        namespace: NAMESPACE.to_string(),
        path,
    })
}

pub unsafe fn store_embedding(key: String, vector: String, metadata: String) -> Result<String, Error> {
    let vector_array: Vec<f32> = serde_json::from_str(&vector)
        .map_err(|e| Error::msg(format!("parse vector error: {}", e)))?;

    #[derive(Serialize)]
    struct Request { namespace: String, key: String, vector: Vec<f32>, metadata: String }
    call_service("vector-store", Request {
        namespace: NAMESPACE.to_string(),
        key,
        vector: vector_array,
        metadata,
    })
}

pub unsafe fn search_by_vector(vector: String, limit: u32) -> Result<String, Error> {
    let vector_array: Vec<f32> = serde_json::from_str(&vector)
        .map_err(|e| Error::msg(format!("parse vector error: {}", e)))?;

    #[derive(Serialize)]
    struct Request { namespace: String, vector: Vec<f32>, limit: u32 }
    call_service("vector-search", Request {
        namespace: NAMESPACE.to_string(),
        vector: vector_array,
        limit,
    })
}
