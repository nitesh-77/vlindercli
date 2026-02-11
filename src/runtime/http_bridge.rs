//! HTTP Bridge — minimal HTTP server for container agent service calls.
//!
//! Provides per-operation endpoints that map to SDK methods:
//!   POST /kv/get, /kv/put, /infer, /embed, etc.
//!
//! The bridge is the JSON-to-typed boundary: it deserializes agent requests
//! into SdkMessage variants, calls typed AgentBridge methods, and serializes
//! the results back to HTTP responses.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::domain::{AgentBridge, SdkMessage};

use super::service_router::ServiceRouter;

/// URL path → SdkMessage `op` field mapping.
fn op_for_path(path: &str) -> Option<&'static str> {
    match path {
        "/kv/get" => Some("kv-get"),
        "/kv/put" => Some("kv-put"),
        "/kv/list" => Some("kv-list"),
        "/kv/delete" => Some("kv-delete"),
        "/vector/store" => Some("vector-store"),
        "/vector/search" => Some("vector-search"),
        "/vector/delete" => Some("vector-delete"),
        "/infer" => Some("infer"),
        "/embed" => Some("embed"),
        "/delegate" => Some("delegate"),
        "/wait" => Some("wait"),
        _ => None,
    }
}

/// A running HTTP bridge server.
///
/// Created by `start()`, runs in a background thread.
/// Shuts down when `stop()` is called or the struct is dropped.
pub(crate) struct HttpBridge {
    port: u16,
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    send_data: Arc<ServiceRouter>,
}

impl HttpBridge {
    /// Start the bridge server in a background thread.
    ///
    /// Binds to `0.0.0.0:0` (OS-assigned port). The container should
    /// connect to `http://host.containers.internal:{port}/`.
    pub(crate) fn start(send_data: Arc<ServiceRouter>) -> std::io::Result<Self> {
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

impl Drop for HttpBridge {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        // Don't join in Drop — the thread will notice the flag on next poll
    }
}

/// Main bridge loop.
fn run_bridge(
    server: tiny_http::Server,
    send_data: Arc<ServiceRouter>,
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

    // Map path to op
    let op = match op_for_path(&path) {
        Some(op) => op,
        None => {
            let body = format!("unknown endpoint: {}", path);
            let response = tiny_http::Response::from_data(body.into_bytes())
                .with_status_code(500);
            let _ = request.respond(response);
            return;
        }
    };

    // Read body
    let mut body = Vec::new();
    if let Err(e) = request.as_reader().read_to_end(&mut body) {
        let msg = format!("body read error: {}", e);
        let response = tiny_http::Response::from_data(msg.into_bytes())
            .with_status_code(500);
        let _ = request.respond(response);
        return;
    }

    // Merge op into body JSON, deserialize, and route to typed methods
    match merge_op(op, &body).and_then(|merged| dispatch(bridge, merged)) {
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

/// Deserialize an SdkMessage and route to typed AgentBridge methods.
///
/// This is the JSON-to-typed boundary: serde on the way in, serde on the way out.
/// Everything between is typed Rust calls on the AgentBridge trait.
fn dispatch(bridge: &dyn AgentBridge, payload: Vec<u8>) -> Result<Vec<u8>, String> {
    let msg: SdkMessage = serde_json::from_slice(&payload)
        .map_err(|e| format!("invalid SDK message: {}", e))?;

    match msg {
        SdkMessage::KvGet { path } => {
            bridge.kv_get(&path)
        }
        SdkMessage::KvPut { path, content } => {
            bridge.kv_put(&path, &content)?;
            Ok(b"ok".to_vec())
        }
        SdkMessage::KvList { path } => {
            let paths = bridge.kv_list(&path)?;
            serde_json::to_vec(&paths)
                .map_err(|e| format!("serialize error: {}", e))
        }
        SdkMessage::KvDelete { path } => {
            let existed = bridge.kv_delete(&path)?;
            Ok(if existed { b"ok".to_vec() } else { b"not_found".to_vec() })
        }
        SdkMessage::VectorStore { key, vector, metadata } => {
            bridge.vector_store(&key, &vector, &metadata)?;
            Ok(b"ok".to_vec())
        }
        SdkMessage::VectorSearch { vector, limit } => {
            let matches = bridge.vector_search(&vector, limit)?;
            serde_json::to_vec(&matches)
                .map_err(|e| format!("serialize error: {}", e))
        }
        SdkMessage::VectorDelete { key } => {
            let existed = bridge.vector_delete(&key)?;
            Ok(if existed { b"ok".to_vec() } else { b"not_found".to_vec() })
        }
        SdkMessage::Infer { model, prompt, max_tokens } => {
            let text = bridge.infer(&model, &prompt, max_tokens)?;
            Ok(text.into_bytes())
        }
        SdkMessage::Embed { model, text } => {
            let vector = bridge.embed(&model, &text)?;
            serde_json::to_vec(&vector)
                .map_err(|e| format!("serialize error: {}", e))
        }
        SdkMessage::Delegate { agent, input } => {
            let handle = bridge.delegate(&agent, &input)?;
            serde_json::to_vec(&serde_json::json!({ "handle": handle }))
                .map_err(|e| format!("serialize error: {}", e))
        }
        SdkMessage::Wait { handle } => {
            let output = bridge.wait(&handle)?;
            serde_json::to_vec(&serde_json::json!({ "output": String::from_utf8_lossy(&output) }))
                .map_err(|e| format!("serialize error: {}", e))
        }
    }
}

/// Merge the `op` field into a JSON object body.
///
/// Input: op = "kv-get", body = `{"path": "/foo"}`
/// Output: `{"op": "kv-get", "path": "/foo"}`
fn merge_op(op: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    let mut map: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(body)
        .map_err(|e| format!("invalid JSON body: {}", e))?;

    map.insert("op".to_string(), serde_json::Value::String(op.to_string()));

    serde_json::to_vec(&map)
        .map_err(|e| format!("JSON serialization error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{InMemoryRegistry, Registry, ResourceId, RuntimeType};
    use crate::queue::{HarnessType, InMemoryQueue, InvokeDiagnostics, InvokeMessage, MessageQueue, SequenceCounter, SessionId, SubmissionId};
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn test_send_data() -> Arc<ServiceRouter> {
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
        Arc::new(ServiceRouter {
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
        let bridge = HttpBridge::start(send_data).unwrap();
        assert!(bridge.port() > 0);
        assert!(bridge.container_url().contains("host.containers.internal"));
        bridge.stop();
    }

    #[test]
    fn bridge_health_check() {
        let send_data = test_send_data();
        let bridge = HttpBridge::start(send_data).unwrap();
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
        let bridge = HttpBridge::start(send_data).unwrap();
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
        let bridge = HttpBridge::start(send_data).unwrap();
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
    fn op_mapping_covers_all_endpoints() {
        assert_eq!(op_for_path("/kv/get"), Some("kv-get"));
        assert_eq!(op_for_path("/kv/put"), Some("kv-put"));
        assert_eq!(op_for_path("/kv/list"), Some("kv-list"));
        assert_eq!(op_for_path("/kv/delete"), Some("kv-delete"));
        assert_eq!(op_for_path("/vector/store"), Some("vector-store"));
        assert_eq!(op_for_path("/vector/search"), Some("vector-search"));
        assert_eq!(op_for_path("/vector/delete"), Some("vector-delete"));
        assert_eq!(op_for_path("/infer"), Some("infer"));
        assert_eq!(op_for_path("/embed"), Some("embed"));
        assert_eq!(op_for_path("/delegate"), Some("delegate"));
        assert_eq!(op_for_path("/wait"), Some("wait"));
        assert_eq!(op_for_path("/health"), None); // health handled separately
        assert_eq!(op_for_path("/unknown"), None);
    }

    #[test]
    fn merge_op_adds_field() {
        let body = br#"{"path": "/foo"}"#;
        let merged = merge_op("kv-get", body).unwrap();
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&merged).unwrap();
        assert_eq!(map["op"], "kv-get");
        assert_eq!(map["path"], "/foo");
    }
}
