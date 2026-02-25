//! Provider server — virtual-host HTTP server that emulates upstream provider APIs.
//!
//! Spawned per invoke, dies when the invoke completes. Self-contained: reads
//! config from env, connects to the registry, checks the agent's requirements,
//! builds its own host table from linked provider crates, and connects to the
//! message queue. Incoming requests are matched against the host table and
//! forwarded to NATS via `call_service()`.
//!
//! The only input is the `InvokeMessage`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tiny_http::{Method, StatusCode};
use vlinder_core::domain::{
    AgentId, ContainerDiagnostics, DelegateDiagnostics, DelegateMessage,
    HttpMethod, InvokeMessage, MessageQueue, Nonce, ObjectStorageType,
    Provider, ProviderHost, ProviderRoute, Registry, RequestDiagnostics,
    RequestMessage, RoutingKey, SequenceCounter, ServiceType,
    VectorStorageType,
};

use crate::config::SidecarConfig;
use crate::factory;

/// A running provider server, scoped to one invoke.
///
/// Shuts down automatically when dropped: the `Drop` impl sets the signal
/// and joins the thread.
pub struct ProviderServer {
    /// Shared flag between this struct and the request loop thread.
    /// Both sides hold an `Arc` to the same `AtomicBool` — the struct
    /// keeps this handle so `Drop` can write `true`, while the thread
    /// holds a clone so it can read the flag each loop iteration.
    /// `Arc` is needed because `std::thread::spawn` moves captured values
    /// into the thread, so a plain reference wouldn't work.
    shutdown_signal: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Shared state between the struct and the request loop.
    /// Updated by kv-put responses, read by the sidecar dispatch loop
    /// for the CompleteMessage.
    state: Arc<RwLock<Option<String>>>,
}

impl ProviderServer {
    /// Start the provider server on port 80.
    ///
    /// Connects to the registry, looks up the agent from `invoke.agent_id`,
    /// checks which providers are needed, connects to the queue, and serves
    /// their hostnames. Always starts — even agents with no provider services
    /// need the server for delegation endpoints.
    pub fn start(invoke: &InvokeMessage) -> Option<Self> {
        let (hosts, queue, registry, invoke, initial_state) = build_context(invoke);

        let server = tiny_http::Server::http("0.0.0.0:80")
            .expect("failed to bind provider server on port 80");
        tracing::info!(event = "provider_server.listening", port = 80, "Provider server started");

        Some(Self::start_request_loop(server, hosts, queue, registry, invoke, initial_state))
    }

    /// Read the final state hash after an invocation completes.
    pub fn final_state(&self) -> Option<String> {
        self.state.read().unwrap().clone()
    }

    /// Spawn the request loop on a background thread.
    ///
    /// Creates the shutdown signal, clones it for the thread, and hands
    /// everything to `request_loop` running on a new thread.
    fn start_request_loop(
        server: tiny_http::Server,
        hosts: Vec<ProviderHost>,
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        invoke: InvokeMessage,
        initial_state: Option<String>,
    ) -> Self {
        let shutdown_signal = Arc::new(AtomicBool::new(false));
        let state = Arc::new(RwLock::new(initial_state));

        // Second handle to the same AtomicBool — this one moves into the
        // thread, while `shutdown_signal` stays on the struct for Drop.
        let should_stop = Arc::clone(&shutdown_signal);
        let state_clone = Arc::clone(&state);

        let thread = std::thread::spawn(move || {
            request_loop(should_stop, server, hosts, queue, registry, invoke, state_clone);
        });

        Self { shutdown_signal, thread: Some(thread), state }
    }
}

/// The main request loop — runs on a background thread for the lifetime of
/// an invoke.
///
/// Each incoming HTTP request is either:
/// - routed to `runtime.vlinder.local` for delegation (delegate/wait), or
/// - matched against the virtual host table and forwarded to the message queue.
///
/// Sequence numbers and state are tracked across requests within the invoke.
/// Runs until `should_stop` is set to `true` or the server encounters an error.
fn request_loop(
    should_stop: Arc<AtomicBool>,
    server: tiny_http::Server,
    hosts: Vec<ProviderHost>,
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    invoke: InvokeMessage,
    state: Arc<RwLock<Option<String>>>,
) {
    let sequence = SequenceCounter::new();
    let mut pending_replies: HashMap<String, RoutingKey> = HashMap::new();

    while !should_stop.load(Ordering::Relaxed) {
        match server.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(mut request)) => {
                let method = request.method().clone();
                let host = extract_host(&request).to_string();
                let path = request.url().to_string();
                tracing::info!(
                    event = "provider_server.request_received",
                    host = %host, path = %path, method = %method,
                    "Provider server received request"
                );

                let mut body = Vec::new();
                let _ = request.as_reader().read_to_end(&mut body);

                let (status, response_body) = if host == "runtime.vlinder.local" {
                    handle_runtime_request(&*queue, &*registry, &invoke, &mut pending_replies, &method, &path, &body)
                } else {
                    match match_route(&hosts, &method, &host, &path, &body) {
                        Ok(route) => forward_to_queue(&*queue, &invoke, &sequence, &state, route, body),
                        Err(err) => err,
                    }
                };

                tracing::info!(
                    event = "provider_server.request_done",
                    status = status, response_bytes = response_body.len(),
                    "Provider server sending response"
                );
                let response = tiny_http::Response::from_data(response_body)
                    .with_status_code(StatusCode(status));
                let _ = request.respond(response);
            }
            Ok(None) => continue,   // timeout, check shutdown flag
            Err(_) => break,         // server error, exit
        }
    }
    tracing::info!(event = "provider_server.stopped", "Provider server stopped");
}

/// Build the provider server context: hosts, queue, registry, and invoke.
///
/// Reads config from env, connects to the registry, looks up the agent,
/// checks its service requirements, and connects to the message queue.
/// Always returns a context — even agents with no provider services need
/// the server for delegation endpoints on `runtime.vlinder.local`.
fn build_context(invoke: &InvokeMessage) -> (Vec<ProviderHost>, Arc<dyn MessageQueue + Send + Sync>, Arc<dyn Registry>, InvokeMessage, Option<String>) {
    let config = SidecarConfig::from_env()
        .expect("failed to parse sidecar config from env");
    let registry = factory::connect_registry(&config.registry_url)
        .expect("failed to connect to registry from provider server");

    let agent = registry.get_agent_by_name(invoke.agent_id.as_str())
        .expect("agent not found in registry");

    let mut hosts = Vec::new();
    let mut initial_state: Option<String> = None;

    let needs_openrouter = agent
        .requirements
        .services
        .get(&ServiceType::Infer)
        .map(|svc| svc.provider == Provider::OpenRouter)
        .unwrap_or(false);

    if needs_openrouter {
        hosts.push(vlinder_infer_openrouter::provider_host());
    }

    let needs_ollama_infer = agent
        .requirements
        .services
        .get(&ServiceType::Infer)
        .map(|svc| svc.provider == Provider::Ollama)
        .unwrap_or(false);

    let needs_ollama_embed = agent
        .requirements
        .services
        .get(&ServiceType::Embed)
        .map(|svc| svc.provider == Provider::Ollama)
        .unwrap_or(false);

    if needs_ollama_infer || needs_ollama_embed {
        hosts.push(vlinder_ollama::provider_host(needs_ollama_infer, needs_ollama_embed));
    }

    let needs_sqlite_vec = agent.vector_storage.as_ref()
        .and_then(|uri| VectorStorageType::from_scheme(uri.scheme()))
        .map(|t| t == VectorStorageType::SqliteVec)
        .unwrap_or(false);

    if needs_sqlite_vec {
        hosts.push(vlinder_sqlite_vec::provider_host());
    }

    // KV storage: any agent with object_storage gets the sqlite-kv provider
    let has_object_storage = agent.object_storage.as_ref()
        .and_then(|uri| ObjectStorageType::from_scheme(uri.scheme()))
        .is_some();

    if has_object_storage {
        hosts.push(vlinder_sqlite_kv::provider_host());
        // Bootstrap state: use invoke.state, or "" if the agent has KV but no state yet
        initial_state = Some(invoke.state.clone().unwrap_or_default());
    }

    let queue = factory::connect_queue(&config.nats_url, &config.state_url)
        .expect("failed to connect to queue from provider server");

    (hosts, queue, registry, invoke.clone(), initial_state)
}

/// Extract the Host header value from a request, or "" if absent.
fn extract_host(request: &tiny_http::Request) -> &str {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("host"))
        .map(|h| h.value.as_str())
        .unwrap_or("")
}

// =========================================================================
// Delegation handlers — runtime.vlinder.local
// =========================================================================

/// Dispatch requests to `runtime.vlinder.local`.
///
/// Routes POST /delegate and POST /wait; everything else is 404.
fn handle_runtime_request(
    queue: &dyn MessageQueue,
    registry: &dyn Registry,
    invoke: &InvokeMessage,
    pending_replies: &mut HashMap<String, RoutingKey>,
    method: &Method,
    path: &str,
    body: &[u8],
) -> (u16, Vec<u8>) {
    if *method == Method::Post && path == "/delegate" {
        handle_delegate(queue, registry, invoke, pending_replies, body)
    } else if *method == Method::Post && path == "/wait" {
        handle_wait(queue, pending_replies, body)
    } else {
        (404, b"not found".to_vec())
    }
}

/// Handle POST /delegate — enqueue a DelegateMessage, return a handle.
///
/// Expects JSON: `{"target": "<agent-name>", "input": "<payload>"}`.
/// Validates the target exists in the registry, builds a DelegateMessage
/// with a fresh nonce, sends it to the queue, and returns the nonce as
/// the delegation handle for a subsequent /wait call.
fn handle_delegate(
    queue: &dyn MessageQueue,
    registry: &dyn Registry,
    invoke: &InvokeMessage,
    pending_replies: &mut HashMap<String, RoutingKey>,
    body: &[u8],
) -> (u16, Vec<u8>) {
    let parsed: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return (400, format!("invalid JSON: {}", e).into_bytes()),
    };

    let target_name = match parsed.get("target").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return (400, b"missing 'target' field".to_vec()),
    };

    let input = match parsed.get("input").and_then(|v| v.as_str()) {
        Some(i) => i,
        None => return (400, b"missing 'input' field".to_vec()),
    };

    // Fast-fail: target must be a registered agent.
    if registry.get_agent_by_name(target_name).is_none() {
        return (404, format!("target agent '{}' not found", target_name).into_bytes());
    }

    let caller = invoke.agent_id.clone();
    let target = AgentId::new(target_name);
    let nonce = Nonce::generate();

    let delegate = DelegateMessage::new(
        invoke.timeline.clone(),
        invoke.submission.clone(),
        invoke.session.clone(),
        caller.clone(),
        target.clone(),
        input.as_bytes().to_vec(),
        nonce.clone(),
        None,
        DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
    );

    let reply_key = delegate.reply_routing_key();
    let handle = nonce.as_str().to_string();

    tracing::info!(
        event = "delegation.sent",
        sha = %invoke.submission,
        caller = %caller, target = %target,
        nonce = %nonce, "Delegating to agent via provider server"
    );

    pending_replies.insert(handle.clone(), reply_key);

    if let Err(e) = queue.send_delegate(delegate) {
        return (502, format!("delegate send error: {}", e).into_bytes());
    }

    let response = format!(r#"{{"handle":"{}"}}"#, handle);
    (200, response.into_bytes())
}

/// Handle POST /wait — block until a delegation reply arrives.
///
/// Expects JSON: `{"handle": "<nonce>"}`.
/// Looks up the reply routing key from `pending_replies`, then polls
/// `receive_delegate_reply` until the target agent's CompleteMessage
/// arrives. Returns the raw output payload.
fn handle_wait(
    queue: &dyn MessageQueue,
    pending_replies: &mut HashMap<String, RoutingKey>,
    body: &[u8],
) -> (u16, Vec<u8>) {
    let parsed: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return (400, format!("invalid JSON: {}", e).into_bytes()),
    };

    let handle = match parsed.get("handle").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => return (400, b"missing 'handle' field".to_vec()),
    };

    let reply_key = match pending_replies.get(&handle) {
        Some(k) => k.clone(),
        None => return (404, format!("unknown delegation handle '{}'", handle).into_bytes()),
    };

    tracing::debug!(handle = %handle, "wait: polling for delegation result");
    let poll_start = std::time::Instant::now();
    let mut poll_count: u64 = 0;

    loop {
        match queue.receive_delegate_reply(&reply_key) {
            Ok((complete, ack)) => {
                let payload = complete.payload.clone();
                let _ = ack();
                pending_replies.remove(&handle);
                tracing::info!(
                    event = "delegation.completed",
                    handle = %handle, polls = poll_count,
                    elapsed = ?poll_start.elapsed(),
                    "Delegation result received via provider server"
                );
                return (200, payload);
            }
            Err(_) => {
                poll_count += 1;
                if poll_count % 100 == 0 {
                    tracing::warn!(
                        handle = %handle, polls = poll_count,
                        elapsed = ?poll_start.elapsed(),
                        "wait: still waiting for delegation result"
                    );
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Match a request against the virtual host table (pure function).
///
/// Matches hostname first, then (method, path) within that host.
/// Validates the request body against the route's declared type.
/// Returns the matched route on success, or an HTTP error tuple on failure.
fn match_route<'a>(
    hosts: &'a [ProviderHost],
    method: &Method,
    host: &str,
    path: &str,
    body: &[u8],
) -> Result<&'a ProviderRoute, (u16, Vec<u8>)> {
    for vhost in hosts {
        if vhost.hostname == host {
            for r in &vhost.routes {
                if to_tiny_method(r.method) == *method && r.path == path {
                    if let Err(e) = (r.validate_request)(body) {
                        return Err((400, e.to_string().into_bytes()));
                    }
                    return Ok(r);
                }
            }
        }
    }
    Err((404, b"not found".to_vec()))
}

/// Forward a matched request to the message queue.
///
/// Builds a `RequestMessage` from the route and invoke context,
/// sends it via `queue.call_service()`, and returns the response.
fn forward_to_queue(
    queue: &dyn MessageQueue,
    invoke: &InvokeMessage,
    sequence: &SequenceCounter,
    state: &RwLock<Option<String>>,
    route: &ProviderRoute,
    body: Vec<u8>,
) -> (u16, Vec<u8>) {
    let seq = sequence.next();
    let received_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let diagnostics = RequestDiagnostics {
        sequence: seq.as_u32(),
        endpoint: format!("/{}", route.service_backend.service_type().as_str()),
        request_bytes: body.len() as u64,
        received_at_ms,
    };

    let request = RequestMessage::new(
        invoke.timeline.clone(),
        invoke.submission.clone(),
        invoke.session.clone(),
        invoke.agent_id.clone(),
        route.service_backend,
        route.operation,
        seq,
        body,
        state.read().unwrap().clone(),
        diagnostics,
    );

    match queue.call_service(request) {
        Ok(response) => {
            if let Some(ref new_state) = response.state {
                *state.write().unwrap() = Some(new_state.clone());
            }
            (response.status_code, response.payload.clone())
        }
        Err(e) => (502, format!("queue error: {}", e).into_bytes()),
    }
}

/// Convert a core `HttpMethod` into the `tiny_http::Method` the server needs.
fn to_tiny_method(m: HttpMethod) -> Method {
    match m {
        HttpMethod::Get => Method::Get,
        HttpMethod::Post => Method::Post,
        HttpMethod::Put => Method::Put,
        HttpMethod::Delete => Method::Delete,
    }
}

impl Drop for ProviderServer {
    fn drop(&mut self) {
        self.shutdown_signal.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hosts() -> Vec<ProviderHost> {
        use vlinder_core::domain::{InferenceBackendType, Operation, ServiceBackend};
        vec![ProviderHost::new("test.vlinder.local", vec![
            ProviderRoute::new::<String, String>(
                HttpMethod::Post,
                "/test",
                ServiceBackend::Infer(InferenceBackendType::Ollama),
                Operation::Run,
            ),
        ])]
    }

    #[test]
    fn matching_route_returns_ok() {
        let hosts = test_hosts();
        assert!(match_route(&hosts, &Method::Post, "test.vlinder.local", "/test", b"\"hello\"").is_ok());
    }

    #[test]
    fn invalid_payload_returns_400() {
        let hosts = test_hosts();
        let err = match_route(&hosts, &Method::Post, "test.vlinder.local", "/test", b"not json").err().unwrap();
        assert_eq!(err.0, 400);
    }

    #[test]
    fn missing_host_returns_404() {
        let hosts = test_hosts();
        let err = match_route(&hosts, &Method::Post, "", "/test", b"\"hello\"").err().unwrap();
        assert_eq!(err.0, 404);
    }

    #[test]
    fn wrong_host_returns_404() {
        let hosts = test_hosts();
        let err = match_route(&hosts, &Method::Post, "other.vlinder.local", "/test", b"\"hello\"").err().unwrap();
        assert_eq!(err.0, 404);
    }

    #[test]
    fn wrong_method_returns_404() {
        let hosts = test_hosts();
        let err = match_route(&hosts, &Method::Get, "test.vlinder.local", "/test", b"").err().unwrap();
        assert_eq!(err.0, 404);
    }

    #[test]
    fn wrong_path_returns_404() {
        let hosts = test_hosts();
        let err = match_route(&hosts, &Method::Post, "test.vlinder.local", "/other", b"\"hello\"").err().unwrap();
        assert_eq!(err.0, 404);
    }
}
