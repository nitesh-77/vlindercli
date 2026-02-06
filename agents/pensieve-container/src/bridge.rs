//! Bridge client — HTTP calls to the Vlinder runtime bridge.
//!
//! Replaces the Extism PDK host functions from the WASM version.
//! All service calls go through VLINDER_BRIDGE_URL.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::io::Read;
use std::sync::OnceLock;

static BRIDGE_URL: OnceLock<String> = OnceLock::new();

fn bridge_url() -> &'static str {
    BRIDGE_URL.get_or_init(|| {
        std::env::var("VLINDER_BRIDGE_URL").unwrap_or_default()
    })
}

fn bridge_call(path: &str, body: &serde_json::Value) -> Result<Vec<u8>, String> {
    let url = format!("{}{}", bridge_url(), path);
    let payload = serde_json::to_string(body)
        .map_err(|e| format!("serialize error: {}", e))?;

    let response = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(|e| format!("bridge call to {} failed: {}", path, e))?;

    let mut buf = Vec::new();
    response.into_reader().read_to_end(&mut buf)
        .map_err(|e| format!("read bridge response: {}", e))?;
    Ok(buf)
}

// ============================================================================
// High-level service functions
// ============================================================================

pub fn infer(model: &str, prompt: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "max_tokens": 512
    });
    let response = bridge_call("/infer", &body)?;
    Ok(String::from_utf8_lossy(&response).to_string())
}

pub fn embed(model: &str, text: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "text": text
    });
    let response = bridge_call("/embed", &body)?;
    Ok(String::from_utf8_lossy(&response).to_string())
}

pub fn get_file(path: &str) -> Result<Vec<u8>, String> {
    let body = serde_json::json!({ "path": path });
    bridge_call("/kv/get", &body)
}

pub fn put_file(path: &str, content: &[u8]) -> Result<String, String> {
    let body = serde_json::json!({
        "path": path,
        "content": BASE64.encode(content)
    });
    let response = bridge_call("/kv/put", &body)?;
    Ok(String::from_utf8_lossy(&response).to_string())
}

pub fn list_files(path: &str) -> Result<String, String> {
    let body = serde_json::json!({ "path": path });
    let response = bridge_call("/kv/list", &body)?;
    Ok(String::from_utf8_lossy(&response).to_string())
}

pub fn store_embedding(key: &str, vector: &str, metadata: &str) -> Result<String, String> {
    let vector_array: Vec<f32> = serde_json::from_str(vector)
        .map_err(|e| format!("parse vector error: {}", e))?;

    let body = serde_json::json!({
        "key": key,
        "vector": vector_array,
        "metadata": metadata
    });
    let response = bridge_call("/vector/store", &body)?;
    Ok(String::from_utf8_lossy(&response).to_string())
}

pub fn search_by_vector(vector: &str, limit: u32) -> Result<String, String> {
    let vector_array: Vec<f32> = serde_json::from_str(vector)
        .map_err(|e| format!("parse vector error: {}", e))?;

    let body = serde_json::json!({
        "vector": vector_array,
        "limit": limit
    });
    let response = bridge_call("/vector/search", &body)?;
    Ok(String::from_utf8_lossy(&response).to_string())
}

/// Fetch a URL directly (container has network access, no bridge needed).
pub fn fetch_url(url: &str) -> Result<String, String> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("fetch {} failed: {}", url, e))?;
    response.into_string()
        .map_err(|e| format!("read response from {}: {}", url, e))
}
