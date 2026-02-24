//! Sidecar's internal HTTP server, exposing platform services to the agent.
//!
//! Uses tiny_http for a fully synchronous server running in a background thread.

use std::sync::Arc;
use serde::Deserialize;
use crate::queue_bridge::QueueBridge;

/// Spawns the HTTP server in a background thread.
pub fn spawn_server(bridge: Arc<QueueBridge>) {
    std::thread::spawn(move || {
        let server = tiny_http::Server::http("0.0.0.0:9000")
            .expect("failed to bind sidecar HTTP server on port 9000");
        tracing::info!(event = "http_server.listening", port = 9000, "Internal sidecar API server started");

        for mut request in server.incoming_requests() {
            let url = request.url().to_string();
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_err() {
                let _ = request.respond(
                    tiny_http::Response::from_string("failed to read request body")
                        .with_status_code(tiny_http::StatusCode(400))
                );
                continue;
            }

            let result: Result<Vec<u8>, String> = match url.as_str() {
                "/services/kv/get" => handle_kv_get(&bridge, &body),
                "/services/kv/put" => handle_kv_put(&bridge, &body),
                "/services/kv/delete" => handle_kv_delete(&bridge, &body),
                "/services/vector/store" => handle_vector_store(&bridge, &body),
                "/services/vector/search" => handle_vector_search(&bridge, &body),
                "/services/vector/delete" => handle_vector_delete(&bridge, &body),
                _ => {
                    let _ = request.respond(
                        tiny_http::Response::from_string("not found")
                            .with_status_code(tiny_http::StatusCode(404))
                    );
                    continue;
                }
            };

            let response = match result {
                Ok(data) => tiny_http::Response::from_data(data),
                Err(e) => tiny_http::Response::from_string(e)
                    .with_status_code(tiny_http::StatusCode(500)),
            };
            let _ = request.respond(response);
        }
    });
}

// =============================================================================
// Request Structs
// =============================================================================

#[derive(Deserialize)]
struct KvGetRequest {
    path: String,
}

#[derive(Deserialize)]
struct KvPutRequest {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct KvDeleteRequest {
    path: String,
}

#[derive(Deserialize)]
struct VectorStoreRequest {
    key: String,
    vector: Vec<f32>,
    metadata: String,
}

#[derive(Deserialize)]
struct VectorSearchRequest {
    vector: Vec<f32>,
    limit: u32,
}

#[derive(Deserialize)]
struct VectorDeleteRequest {
    key: String,
}

// =============================================================================
// Handlers
// =============================================================================

fn handle_kv_get(bridge: &QueueBridge, body: &str) -> Result<Vec<u8>, String> {
    let req: KvGetRequest = serde_json::from_str(body)
        .map_err(|e| format!("parse error: {}", e))?;
    bridge.kv_get(&req.path)
}

fn handle_kv_put(bridge: &QueueBridge, body: &str) -> Result<Vec<u8>, String> {
    let req: KvPutRequest = serde_json::from_str(body)
        .map_err(|e| format!("parse error: {}", e))?;
    bridge.kv_put(&req.path, &req.content)?;
    Ok(b"OK".to_vec())
}

fn handle_kv_delete(bridge: &QueueBridge, body: &str) -> Result<Vec<u8>, String> {
    let req: KvDeleteRequest = serde_json::from_str(body)
        .map_err(|e| format!("parse error: {}", e))?;
    let deleted = bridge.kv_delete(&req.path)?;
    serde_json::to_vec(&deleted).map_err(|e| format!("serialize error: {}", e))
}

fn handle_vector_store(bridge: &QueueBridge, body: &str) -> Result<Vec<u8>, String> {
    let req: VectorStoreRequest = serde_json::from_str(body)
        .map_err(|e| format!("parse error: {}", e))?;
    bridge.vector_store(&req.key, &req.vector, &req.metadata)?;
    Ok(b"OK".to_vec())
}

fn handle_vector_search(bridge: &QueueBridge, body: &str) -> Result<Vec<u8>, String> {
    let req: VectorSearchRequest = serde_json::from_str(body)
        .map_err(|e| format!("parse error: {}", e))?;
    let matches = bridge.vector_search(&req.vector, req.limit)?;
    serde_json::to_vec(&matches).map_err(|e| format!("serialize error: {}", e))
}

fn handle_vector_delete(bridge: &QueueBridge, body: &str) -> Result<Vec<u8>, String> {
    let req: VectorDeleteRequest = serde_json::from_str(body)
        .map_err(|e| format!("parse error: {}", e))?;
    let deleted = bridge.vector_delete(&req.key)?;
    serde_json::to_vec(&deleted).map_err(|e| format!("serialize error: {}", e))
}

