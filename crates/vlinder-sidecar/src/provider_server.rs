//! Provider server — virtual-host HTTP server that emulates upstream provider APIs.
//!
//! Binds port 80 inside the pod so the agent container can reach provider
//! endpoints (e.g. `openrouter.vlinder.local`) without holding real credentials.
//! Routes are configured per virtual host: each hostname carries its own set
//! of (method, path) → handler mappings.

use tiny_http::{Method, StatusCode};

/// Handler function: takes request body, returns (status, response body).
pub type Handler = Box<dyn Fn(&[u8]) -> (u16, Vec<u8>) + Send + Sync>;

/// A single route: method + path + handler.
pub struct Route {
    pub method: Method,
    pub path: String,
    pub handler: Handler,
}

/// A virtual host: hostname + its routes.
pub struct VirtualHost {
    pub hostname: String,
    pub routes: Vec<Route>,
}

impl Route {
    pub fn new(
        method: Method,
        path: impl Into<String>,
        handler: impl Fn(&[u8]) -> (u16, Vec<u8>) + Send + Sync + 'static,
    ) -> Self {
        Self { method, path: path.into(), handler: Box::new(handler) }
    }
}

impl VirtualHost {
    pub fn new(hostname: impl Into<String>, routes: Vec<Route>) -> Self {
        Self { hostname: hostname.into(), routes }
    }
}

/// OpenRouter virtual host: GET / → 200 "ok".
pub fn openrouter_host() -> VirtualHost {
    VirtualHost::new("openrouter.vlinder.local", vec![
        Route::new(Method::Get, "/", |_| (200, b"ok".to_vec())),
    ])
}

/// Spawn the provider HTTP server on port 80 in a background thread.
pub fn spawn_provider_server(hosts: Vec<VirtualHost>) {
    let server = tiny_http::Server::http("0.0.0.0:80")
        .expect("failed to bind provider server on port 80");
    tracing::info!(event = "provider_server.listening", port = 80, "Provider server started");

    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let host = extract_host(&request).to_string();
            let path = request.url().to_string();
            let mut body = Vec::new();
            let _ = request.as_reader().read_to_end(&mut body);

            let (status, response_body) = route(&hosts, request.method(), &host, &path, &body);
            let response = tiny_http::Response::from_data(response_body)
                .with_status_code(StatusCode(status));
            let _ = request.respond(response);
        }
    });
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
/// Returns 404 if no match is found.
fn route(hosts: &[VirtualHost], method: &Method, host: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    for vhost in hosts {
        if vhost.hostname == host {
            for r in &vhost.routes {
                if r.method == *method && r.path == path {
                    return (r.handler)(body);
                }
            }
        }
    }
    (404, b"not found".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hosts() -> Vec<VirtualHost> {
        vec![openrouter_host()]
    }

    #[test]
    fn get_root_openrouter_returns_200() {
        let hosts = test_hosts();
        let (status, body) = route(&hosts, &Method::Get, "openrouter.vlinder.local", "/", b"");
        assert_eq!(status, 200);
        assert_eq!(body, b"ok");
    }

    #[test]
    fn get_root_without_host_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Get, "", "/", b"");
        assert_eq!(status, 404);
    }

    #[test]
    fn get_root_wrong_host_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Get, "invalid.vlinder.local", "/", b"");
        assert_eq!(status, 404);
    }

    #[test]
    fn post_openrouter_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "openrouter.vlinder.local", "/", b"");
        assert_eq!(status, 404);
    }

    #[test]
    fn get_unknown_path_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Get, "openrouter.vlinder.local", "/v1/chat/completions", b"");
        assert_eq!(status, 404);
    }
}
