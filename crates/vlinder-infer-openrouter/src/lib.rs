//! OpenRouter provider — declares the hostname and routes
//! for the OpenRouter inference backend.

use async_openai::types::chat::{CreateChatCompletionRequest, CreateChatCompletionResponse};
use vlinder_core::domain::{HttpMethod, ProviderHost, ProviderRoute};

/// The virtual hostname the sidecar will serve for OpenRouter.
pub const HOSTNAME: &str = "openrouter.vlinder.local";

/// Build the provider host declaration for OpenRouter.
pub fn provider_host() -> ProviderHost {
    ProviderHost::new(HOSTNAME, vec![
        ProviderRoute::new::<CreateChatCompletionRequest, CreateChatCompletionResponse>(
            HttpMethod::Post, "/v1/chat/completions",
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_is_openrouter_vlinder_local() {
        assert_eq!(HOSTNAME, "openrouter.vlinder.local");
    }

    #[test]
    fn provider_host_has_correct_hostname() {
        let host = provider_host();
        assert_eq!(host.hostname, "openrouter.vlinder.local");
    }

    #[test]
    fn provider_host_has_one_route() {
        let host = provider_host();
        assert_eq!(host.routes.len(), 1);
    }

    #[test]
    fn route_is_post_chat_completions() {
        let host = provider_host();
        let route = &host.routes[0];
        assert_eq!(route.method, HttpMethod::Post);
        assert_eq!(route.path, "/v1/chat/completions");
    }

    #[test]
    fn rejects_invalid_request() {
        let host = provider_host();
        let route = &host.routes[0];
        let result = (route.validate_request)(b"not json");
        assert!(result.is_err());
    }

    #[test]
    fn accepts_valid_request() {
        let host = provider_host();
        let route = &host.routes[0];
        let body = serde_json::json!({
            "model": "anthropic/claude-sonnet-4",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let result = (route.validate_request)(&bytes);
        assert!(result.is_ok());
    }
}
