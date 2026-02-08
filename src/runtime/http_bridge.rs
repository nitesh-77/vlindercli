//! HTTP Bridge — minimal HTTP server for container agent service calls.
//!
//! Provides per-operation endpoints that map to SDK methods:
//!   POST /kv/get, /kv/put, /infer, /embed, etc.
//!
//! The bridge merges the URL path into an `op` field in the request body,
//! creating a full SdkMessage JSON that feeds into `ServiceRouter::dispatch`. This keeps
//! SdkMessage as the single routing truth while giving agents a clean REST API.
//!
//! Zero external dependencies — uses `std::net::TcpListener` only.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

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
        let listener = TcpListener::bind("0.0.0.0:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stop_flag);

        let bridge_data = Arc::clone(&send_data);
        let handle = std::thread::spawn(move || {
            run_bridge(listener, bridge_data, stop);
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

    /// The port the bridge is listening on.
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// The URL the container should use to reach this bridge.
    pub(crate) fn container_url(&self) -> String {
        format!("http://host.containers.internal:{}", self.port)
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

/// Main bridge loop. Accepts connections, handles one request per connection.
fn run_bridge(
    listener: TcpListener,
    send_data: Arc<ServiceRouter>,
    stop: Arc<AtomicBool>,
) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match listener.accept() {
            Ok((stream, _addr)) => {
                let _ = stream.set_nonblocking(false);
                handle_connection(stream, &send_data);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Err(_) => break,
        }
    }
}

/// Handle a single HTTP connection.
fn handle_connection(mut stream: TcpStream, send_data: &ServiceRouter) {
    let result = parse_and_handle(&stream, send_data);

    match result {
        Ok(body) => write_response(&mut stream, 200, &body),
        Err(msg) => write_response(&mut stream, 500, msg.as_bytes()),
    }
}

/// Parse the HTTP request and delegate to dispatch.
fn parse_and_handle(
    stream: &TcpStream,
    send_data: &ServiceRouter,
) -> Result<Vec<u8>, String> {
    let mut reader = BufReader::new(stream);

    // Read request line: "POST /kv/get HTTP/1.1" or "GET /health HTTP/1.1"
    let mut request_line = String::new();
    reader.read_line(&mut request_line)
        .map_err(|e| format!("read error: {}", e))?;

    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return Err("malformed request line".to_string());
    }
    let method = parts[0];
    let path = parts[1];

    // Health check — no body needed
    if method == "GET" && path == "/health" {
        // Read remaining headers to consume the request
        drain_headers(&mut reader)?;
        return Ok(b"ok".to_vec());
    }

    if method != "POST" {
        drain_headers(&mut reader)?;
        return Err(format!("expected POST, got {}", method));
    }

    // Map path to op
    let op = op_for_path(path)
        .ok_or_else(|| format!("unknown endpoint: {}", path))?;

    // Read headers, extract Content-Length
    let content_length = read_content_length(&mut reader)?;

    if content_length == 0 {
        return Err("missing or zero Content-Length".to_string());
    }

    // Read body
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)
        .map_err(|e| format!("body read error: {}", e))?;

    // Merge op into body JSON
    let merged = merge_op(op, &body)?;

    // Delegate to the shared dispatch
    send_data.dispatch(merged)
}

/// Read headers and extract Content-Length. Consumes all headers up to the blank line.
fn read_content_length(reader: &mut BufReader<&TcpStream>) -> Result<usize, String> {
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)
            .map_err(|e| format!("header read error: {}", e))?;

        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }

        let lower = trimmed.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse()
                .map_err(|_| "invalid Content-Length".to_string())?;
        }
    }
    Ok(content_length)
}

/// Read and discard headers until the blank line.
fn drain_headers(reader: &mut BufReader<&TcpStream>) -> Result<(), String> {
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)
            .map_err(|e| format!("header read error: {}", e))?;
        if line.trim().is_empty() {
            break;
        }
    }
    Ok(())
}

/// Merge the `op` field into a JSON object body.
///
/// Input: op = "kv-get", body = `{"path": "/foo"}`
/// Output: `{"op": "kv-get", "path": "/foo"}`
fn merge_op(op: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    let mut map: HashMap<String, serde_json::Value> = serde_json::from_slice(body)
        .map_err(|e| format!("invalid JSON body: {}", e))?;

    map.insert("op".to_string(), serde_json::Value::String(op.to_string()));

    serde_json::to_vec(&map)
        .map_err(|e| format!("JSON serialization error: {}", e))
}

/// Write a minimal HTTP response.
fn write_response(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let status_text = match status {
        200 => "OK",
        500 => "Internal Server Error",
        _ => "Error",
    };

    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, status_text, body.len()
    );

    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ResourceId, RuntimeType};
    use crate::queue::{HarnessType, InMemoryQueue, InvokeMessage, MessageQueue, SequenceCounter, SessionId, SubmissionId};

    fn test_send_data() -> Arc<ServiceRouter> {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let invoke = InvokeMessage::new(
            SubmissionId::new(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            ResourceId::new("http://127.0.0.1:9000/agents/test"),
            b"test".to_vec(),
        );
        Arc::new(ServiceRouter {
            queue,
            invoke: std::sync::RwLock::new(invoke),
            kv_backend: None,
            vec_backend: None,
            model_backends: std::collections::HashMap::new(),
            sequence: SequenceCounter::new(),
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
        stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();

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
            "POST /unknown HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
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
            "POST /kv/get HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
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
        assert_eq!(op_for_path("/health"), None); // health handled separately
        assert_eq!(op_for_path("/unknown"), None);
    }

    #[test]
    fn merge_op_adds_field() {
        let body = br#"{"path": "/foo"}"#;
        let merged = merge_op("kv-get", body).unwrap();
        let map: HashMap<String, serde_json::Value> = serde_json::from_slice(&merged).unwrap();
        assert_eq!(map["op"], "kv-get");
        assert_eq!(map["path"], "/foo");
    }
}
