//! Provider server — virtual-host HTTP server that emulates upstream provider APIs.
//!
//! Binds port 80 inside the pod so the agent container can reach provider
//! endpoints (e.g. `openrouter.vlinder.local`) without holding real credentials.
//!
//! Provider crates declare their routes via `ProviderHost` (from vlinder-core).
//! This module is pure plumbing — it matches incoming requests against the
//! declared routes and converts `HttpMethod` to `tiny_http::Method`.

use std::io::Read;

use tiny_http::{Method, StatusCode};
use vlinder_core::domain::{HttpMethod, ProviderHost};

/// Convert a core `HttpMethod` into the `tiny_http::Method` the server needs.
fn to_tiny_method(m: HttpMethod) -> Method {
    match m {
        HttpMethod::Get => Method::Get,
        HttpMethod::Post => Method::Post,
        HttpMethod::Put => Method::Put,
        HttpMethod::Delete => Method::Delete,
    }
}

/// Spawn the provider HTTP server on port 80 in a background thread.
pub fn spawn_provider_server(hosts: Vec<ProviderHost>) {
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

    fn test_hosts() -> Vec<ProviderHost> {
        vec![vlinder_infer_openrouter::provider_host()]
    }

    #[test]
    fn post_root_valid_string_returns_200() {
        let hosts = test_hosts();
        let (status, body) = route(&hosts, &Method::Post, "openrouter.vlinder.local", "/", b"\"hello\"");
        assert_eq!(status, 200);
        assert_eq!(body, b"ok");
    }

    #[test]
    fn post_root_invalid_json_returns_400() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "openrouter.vlinder.local", "/", b"not json");
        assert_eq!(status, 400);
    }

    #[test]
    fn post_without_host_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "", "/", b"\"hello\"");
        assert_eq!(status, 404);
    }

    #[test]
    fn post_wrong_host_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "invalid.vlinder.local", "/", b"\"hello\"");
        assert_eq!(status, 404);
    }

    #[test]
    fn get_openrouter_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Get, "openrouter.vlinder.local", "/", b"");
        assert_eq!(status, 404);
    }

    #[test]
    fn post_unknown_path_returns_404() {
        let hosts = test_hosts();
        let (status, _) = route(&hosts, &Method::Post, "openrouter.vlinder.local", "/v1/chat/completions", b"\"hello\"");
        assert_eq!(status, 404);
    }
}
