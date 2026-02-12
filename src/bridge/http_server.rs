//! HTTP Bridge Server — minimal HTTP server for container agent service calls.
//!
//! Provides per-operation endpoints that map to SDK methods:
//!   POST /kv/get, /kv/put, /infer, /embed, etc.
//!
//! The bridge is the JSON-to-typed boundary: it deserializes request fields
//! from JSON, calls typed AgentBridge methods, and serializes the results
//! back to HTTP responses.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::domain::AgentBridge;

use super::http_bridge::HttpBridge;

/// A running HTTP bridge server.
///
/// Created by `start()`, runs in a background thread.
/// Shuts down when `stop()` is called or the struct is dropped.
pub(crate) struct HttpBridgeServer {
    port: u16,
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    send_data: Arc<HttpBridge>,
}

impl HttpBridgeServer {
    /// Start the bridge server in a background thread.
    ///
    /// Binds to `0.0.0.0:0` (OS-assigned port). The container should
    /// connect to `http://host.containers.internal:{port}/`.
    pub(crate) fn start(send_data: Arc<HttpBridge>) -> std::io::Result<Self> {
        let server = tiny_http::Server::http("0.0.0.0:0")
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let port = server.server_addr().to_ip().unwrap().port();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stop_flag);

        let bridge_data = Arc::clone(&send_data);
        let handle = std::thread::spawn(move || {
            run_bridge(server, bridge_data, stop);
        });

        Ok(Self {
            port,
            stop_flag,
            handle: Some(handle),
            send_data,
        })
    }

    /// Update the invoke context for a new invocation.
    pub(crate) fn update_invoke(&self, invoke: crate::queue::InvokeMessage) {
        self.send_data.update_invoke(invoke);
    }

    /// Read the final state hash after an invocation completes (ADR 055).
    pub(crate) fn final_state(&self) -> Option<String> {
        self.send_data.final_state()
    }

    /// The port the bridge is listening on.
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// The URL the container should use to reach this bridge.
    pub(crate) fn container_url(&self) -> String {
        format!("http://host.containers.internal:{}", self.port())
    }

    /// Signal the bridge to stop and wait for it to finish.
    pub(crate) fn stop(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for HttpBridgeServer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        // Don't join in Drop — the thread will notice the flag on next poll
    }
}

/// Main bridge loop.
fn run_bridge(
    server: tiny_http::Server,
    send_data: Arc<HttpBridge>,
    stop: Arc<AtomicBool>,
) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Non-blocking receive with timeout so we can check the stop flag
        let request = match server.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(Some(req)) => req,
            Ok(None) => continue,    // timeout, check stop flag
            Err(_) => break,
        };

        handle_request(request, &*send_data);
    }
}

/// Handle a single HTTP request.
fn handle_request(mut request: tiny_http::Request, bridge: &dyn AgentBridge) {
    let method = request.method().clone();
    let path = request.url().to_string();

    // Health check
    if method == tiny_http::Method::Get && path == "/health" {
        let response = tiny_http::Response::from_data(b"ok".to_vec());
        let _ = request.respond(response);
        return;
    }

    if method != tiny_http::Method::Post {
        let body = format!("expected POST, got {}", method);
        let response = tiny_http::Response::from_data(body.into_bytes())
            .with_status_code(500);
        let _ = request.respond(response);
        return;
    }

    // Read body
    let mut body = Vec::new();
    if let Err(e) = request.as_reader().read_to_end(&mut body) {
        let msg = format!("body read error: {}", e);
        let response = tiny_http::Response::from_data(msg.into_bytes())
            .with_status_code(500);
        let _ = request.respond(response);
        return;
    }

    // Route directly from URL path to typed AgentBridge methods
    match route(&path, &body, bridge) {
        Ok(result) => {
            let response = tiny_http::Response::from_data(result);
            let _ = request.respond(response);
        }
        Err(msg) => {
            let response = tiny_http::Response::from_data(msg.into_bytes())
                .with_status_code(500);
            let _ = request.respond(response);
        }
    }
}

/// Route a request to the appropriate typed AgentBridge method.
///
/// This is the JSON-to-typed boundary. The URL path determines the operation;
/// the JSON body carries the operation-specific fields.
fn route(path: &str, body: &[u8], bridge: &dyn AgentBridge) -> Result<Vec<u8>, String> {
    let json: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("invalid JSON: {}", e))?;

    match path {
        "/kv/get" => {
            let path = json_str(&json, "path")?;
            bridge.kv_get(path)
        }
        "/kv/put" => {
            let path = json_str(&json, "path")?;
            let content = json_str(&json, "content")?;
            bridge.kv_put(path, content)?;
            Ok(b"ok".to_vec())
        }
        "/kv/list" => {
            let path = json_str(&json, "path")?;
            let paths = bridge.kv_list(path)?;
            serde_json::to_vec(&paths)
                .map_err(|e| format!("serialize error: {}", e))
        }
        "/kv/delete" => {
            let path = json_str(&json, "path")?;
            let existed = bridge.kv_delete(path)?;
            Ok(if existed { b"ok".to_vec() } else { b"not_found".to_vec() })
        }
        "/vector/store" => {
            let key = json_str(&json, "key")?;
            let vector = json_f32_array(&json, "vector")?;
            let metadata = json_str(&json, "metadata")?;
            bridge.vector_store(key, &vector, metadata)?;
            Ok(b"ok".to_vec())
        }
        "/vector/search" => {
            let vector = json_f32_array(&json, "vector")?;
            let limit = json_u32(&json, "limit")?;
            let matches = bridge.vector_search(&vector, limit)?;
            serde_json::to_vec(&matches)
                .map_err(|e| format!("serialize error: {}", e))
        }
        "/vector/delete" => {
            let key = json_str(&json, "key")?;
            let existed = bridge.vector_delete(key)?;
            Ok(if existed { b"ok".to_vec() } else { b"not_found".to_vec() })
        }
        "/infer" => {
            let model = json_str(&json, "model")?;
            let prompt = json_str(&json, "prompt")?;
            let max_tokens = json_u32(&json, "max_tokens")?;
            let text = bridge.infer(model, prompt, max_tokens)?;
            Ok(text.into_bytes())
        }
        "/embed" => {
            let model = json_str(&json, "model")?;
            let text = json_str(&json, "text")?;
            let vector = bridge.embed(model, text)?;
            serde_json::to_vec(&vector)
                .map_err(|e| format!("serialize error: {}", e))
        }
        "/delegate" => {
            let agent = json_str(&json, "agent")?;
            let input = json_str(&json, "input")?;
            let handle = bridge.delegate(agent, input)?;
            serde_json::to_vec(&serde_json::json!({ "handle": handle }))
                .map_err(|e| format!("serialize error: {}", e))
        }
        "/wait" => {
            let handle = json_str(&json, "handle")?;
            let output = bridge.wait(handle)?;
            serde_json::to_vec(&serde_json::json!({ "output": String::from_utf8_lossy(&output) }))
                .map_err(|e| format!("serialize error: {}", e))
        }
        _ => Err(format!("unknown endpoint: {}", path)),
    }
}

/// Extract a string field from a JSON value.
fn json_str<'a>(json: &'a serde_json::Value, field: &str) -> Result<&'a str, String> {
    json.get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing or non-string field: {}", field))
}

/// Extract a u32 field from a JSON value.
fn json_u32(json: &serde_json::Value, field: &str) -> Result<u32, String> {
    json.get(field)
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .ok_or_else(|| format!("missing or non-integer field: {}", field))
}

/// Extract a Vec<f32> field from a JSON value.
fn json_f32_array(json: &serde_json::Value, field: &str) -> Result<Vec<f32>, String> {
    json.get(field)
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("missing or non-array field: {}", field))?
        .iter()
        .map(|v| v.as_f64().map(|f| f as f32).ok_or_else(|| format!("non-numeric value in {}", field)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{InMemoryRegistry, Registry, ResourceId, RuntimeType};
    use crate::queue::{HarnessType, InMemoryQueue, InvokeDiagnostics, InvokeMessage, MessageQueue, SequenceCounter, SessionId, SubmissionId};
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn test_send_data() -> Arc<HttpBridge> {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry: Arc<dyn Registry> = Arc::new(InMemoryRegistry::new());
        let invoke = InvokeMessage::new(
            SubmissionId::new(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            ResourceId::new("http://127.0.0.1:9000/agents/test"),
            b"test".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );
        Arc::new(HttpBridge {
            queue,
            registry,
            invoke: std::sync::RwLock::new(invoke),
            kv_backend: None,
            vec_backend: None,
            model_backends: std::collections::HashMap::new(),
            sequence: SequenceCounter::new(),
            current_state: std::sync::RwLock::new(None),
        })
    }

    #[test]
    fn bridge_starts_and_stops() {
        let send_data = test_send_data();
        let bridge = HttpBridgeServer::start(send_data).unwrap();
        assert!(bridge.port() > 0);
        assert!(bridge.container_url().contains("host.containers.internal"));
        bridge.stop();
    }

    #[test]
    fn bridge_health_check() {
        let send_data = test_send_data();
        let bridge = HttpBridgeServer::start(send_data).unwrap();
        let port = bridge.port();

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        assert!(response.contains("ok"));

        bridge.stop();
    }

    #[test]
    fn bridge_rejects_unknown_path() {
        let send_data = test_send_data();
        let bridge = HttpBridgeServer::start(send_data).unwrap();
        let port = bridge.port();

        let body = b"{}";
        let request = format!(
            "POST /unknown HTTP/1.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap()
        );

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("500"));
        assert!(response.contains("unknown endpoint"));

        bridge.stop();
    }

    #[test]
    fn bridge_rejects_kv_without_backend() {
        let send_data = test_send_data(); // no kv_backend configured
        let bridge = HttpBridgeServer::start(send_data).unwrap();
        let port = bridge.port();

        let body = br#"{"path": "/test.txt"}"#;
        let request = format!(
            "POST /kv/get HTTP/1.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap()
        );

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("500"));
        assert!(response.contains("no object_storage configured"));

        bridge.stop();
    }

    #[test]
    fn json_str_extracts_field() {
        let json: serde_json::Value = serde_json::from_str(r#"{"path": "/foo"}"#).unwrap();
        assert_eq!(json_str(&json, "path").unwrap(), "/foo");
        assert!(json_str(&json, "missing").is_err());
    }

    #[test]
    fn json_u32_extracts_field() {
        let json: serde_json::Value = serde_json::from_str(r#"{"limit": 10}"#).unwrap();
        assert_eq!(json_u32(&json, "limit").unwrap(), 10);
        assert!(json_u32(&json, "missing").is_err());
    }

    #[test]
    fn json_f32_array_extracts_field() {
        let json: serde_json::Value = serde_json::from_str(r#"{"vector": [1.0, 2.5, 3.0]}"#).unwrap();
        let v = json_f32_array(&json, "vector").unwrap();
        assert_eq!(v, vec![1.0, 2.5, 3.0]);
        assert!(json_f32_array(&json, "missing").is_err());
    }
}
