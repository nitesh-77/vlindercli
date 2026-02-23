//! Provider server — virtual-host HTTP server that emulates upstream provider APIs.
//!
//! Spawned per invoke, dies when the invoke completes. Self-contained: reads
//! config from env, connects to the registry, checks the agent's requirements,
//! and builds its own host table from linked provider crates.
//!
//! The only input is the `InvokeMessage`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tiny_http::{Method, StatusCode};
use vlinder_core::domain::{HttpMethod, InvokeMessage, Provider, ProviderHost, ServiceType};

use crate::config::SidecarConfig;
use crate::factory;

/// A running provider server, scoped to one invoke.
///
/// Shuts down automatically when dropped.
pub struct ProviderServer {
    shutdown: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ProviderServer {
    /// Start the provider server on port 80.
    ///
    /// Connects to the registry, looks up the agent from `invoke.agent_id`,
    /// checks which providers are needed, and serves their hostnames.
    /// Returns `None` if the agent doesn't need any providers.
    pub fn start(invoke: &InvokeMessage) -> Option<Self> {
        let hosts = discover_hosts(invoke);
        if hosts.is_empty() {
            return None;
        }

        let server = tiny_http::Server::http("0.0.0.0:80")
            .expect("failed to bind provider server on port 80");
        tracing::info!(event = "provider_server.listening", port = 80, "Provider server started");

        Some(Self::spawn(server, hosts))
    }

    /// Spawn the request-handling thread.
    fn spawn(server: tiny_http::Server, hosts: Vec<ProviderHost>) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_flag = Arc::clone(&shutdown);

        let thread = std::thread::spawn(move || {
            while !shutdown_flag.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(mut request)) => {
                        let host = extract_host(&request).to_string();
                        let path = request.url().to_string();

                        let mut body = Vec::new();
                        let _ = request.as_reader().read_to_end(&mut body);

                        let (status, response_body) = route(&hosts, request.method(), &host, &path, &body);
                        let response = tiny_http::Response::from_data(response_body)
                            .with_status_code(StatusCode(status));
                        let _ = request.respond(response);
                    }
                    Ok(None) => continue,   // timeout, check shutdown flag
                    Err(_) => break,         // server error, exit
                }
            }
            tracing::info!(event = "provider_server.stopped", "Provider server stopped");
        });

        Self { shutdown, thread: Some(thread) }
    }
}

/// Discover which provider hosts the agent needs.
///
/// Reads config from env, connects to the registry, looks up the agent,
/// and checks its service requirements against linked provider crates.
fn discover_hosts(invoke: &InvokeMessage) -> Vec<ProviderHost> {
    let config = SidecarConfig::from_env()
        .expect("failed to parse sidecar config from env");
    let registry = factory::connect_registry(&config.registry_url)
        .expect("failed to connect to registry from provider server");

    let agent = registry.get_agent_by_name(invoke.agent_id.as_str())
        .expect("agent not found in registry");

    let mut hosts = Vec::new();

    let needs_openrouter = agent
        .requirements
        .services
        .get(&ServiceType::Infer)
        .map(|svc| svc.provider == Provider::OpenRouter)
        .unwrap_or(false);

    if needs_openrouter {
        hosts.push(vlinder_infer_openrouter::provider_host());
    }

    hosts
}

impl Drop for ProviderServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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

/// Extract the Host header value from a request, or "" if absent.
fn extract_host(request: &tiny_http::Request) -> &str {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("host"))
        .map(|h| h.value.as_str())
        .unwrap_or("")
}

/// Route a request through the virtual host table.
///
/// Matches hostname first, then (method, path) within that host.
/// Validates the request body against the route's declared type.
/// Returns 404 if no match, 400 if validation fails.
fn route(hosts: &[ProviderHost], method: &Method, host: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    for vhost in hosts {
        if vhost.hostname == host {
            for r in &vhost.routes {
                if to_tiny_method(r.method) == *method && r.path == path {
                    if let Err(e) = (r.validate_request)(body) {
                        return (400, e.to_string().into_bytes());
                    }
                    return (200, b"ok".to_vec());
                }
            }
        }
    }
    (404, b"not found".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vlinder_core::domain::ProviderRoute;

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
    fn matching_route_returns_200() {
        let hosts = test_hosts();
        let (status, body) = route(&hosts, &Method::Post, "test.vlinder.local", "/test", b"\"hello\"");
        assert_eq!(status, 200);
        assert_eq!(body, b"ok");
    }

    #[test]
    fn invalid_payload_returns_400() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "test.vlinder.local", "/test", b"not json");
        assert_eq!(status, 400);
    }

    #[test]
    fn missing_host_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "", "/test", b"\"hello\"");
        assert_eq!(status, 404);
    }

    #[test]
    fn wrong_host_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "other.vlinder.local", "/test", b"\"hello\"");
        assert_eq!(status, 404);
    }

    #[test]
    fn wrong_method_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Get, "test.vlinder.local", "/test", b"");
        assert_eq!(status, 404);
    }

    #[test]
    fn wrong_path_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "test.vlinder.local", "/other", b"\"hello\"");
        assert_eq!(status, 404);
    }
}
