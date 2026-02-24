//! Ollama provider — declares the hostname and routes
//! for the Ollama inference backend, and the worker that
//! processes inference requests.
//!
//! Three endpoints:
//! - `/v1/chat/completions` — OpenAI-compatible (Operation::Run)
//! - `/api/chat` — Ollama native chat (Operation::Chat)
//! - `/api/generate` — Ollama native generate (Operation::Generate)

mod types;
mod worker;

pub use types::{
    OllamaChatRequest, OllamaChatResponse,
    OllamaGenerateRequest, OllamaGenerateResponse,
};
pub use worker::OllamaWorker;

use async_openai::types::chat::{CreateChatCompletionRequest, CreateChatCompletionResponse};
use vlinder_core::domain::{HttpMethod, InferenceBackendType, Operation, ProviderHost, ProviderRoute, ServiceBackend};

/// The virtual hostname the sidecar will serve for Ollama.
pub const HOSTNAME: &str = "ollama.vlinder.local";

/// Build the provider host declaration for Ollama.
pub fn provider_host() -> ProviderHost {
    let backend = ServiceBackend::Infer(InferenceBackendType::Ollama);

    ProviderHost::new(HOSTNAME, vec![
        ProviderRoute::new::<CreateChatCompletionRequest, CreateChatCompletionResponse>(
            HttpMethod::Post,
            "/v1/chat/completions",
            backend,
            Operation::Run,
        ),
        ProviderRoute::new::<OllamaChatRequest, OllamaChatResponse>(
            HttpMethod::Post,
            "/api/chat",
            backend,
            Operation::Chat,
        ),
        ProviderRoute::new::<OllamaGenerateRequest, OllamaGenerateResponse>(
            HttpMethod::Post,
            "/api/generate",
            backend,
            Operation::Generate,
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_is_ollama_vlinder_local() {
        assert_eq!(HOSTNAME, "ollama.vlinder.local");
    }

    #[test]
    fn provider_host_has_correct_hostname() {
        let host = provider_host();
        assert_eq!(host.hostname, "ollama.vlinder.local");
    }

    #[test]
    fn provider_host_has_three_routes() {
        let host = provider_host();
        assert_eq!(host.routes.len(), 3);
    }

    // --- /v1/chat/completions (OpenAI-compatible) ---

    #[test]
    fn openai_route_is_post_chat_completions() {
        let host = provider_host();
        let route = &host.routes[0];
        assert_eq!(route.method, HttpMethod::Post);
        assert_eq!(route.path, "/v1/chat/completions");
    }

    #[test]
    fn openai_route_has_correct_backend_and_operation() {
        let host = provider_host();
        let route = &host.routes[0];
        assert_eq!(route.service_backend, ServiceBackend::Infer(InferenceBackendType::Ollama));
        assert_eq!(route.operation, Operation::Run);
    }

    #[test]
    fn openai_route_rejects_invalid_request() {
        let host = provider_host();
        let route = &host.routes[0];
        assert!((route.validate_request)(b"not json").is_err());
    }

    #[test]
    fn openai_route_accepts_valid_request() {
        let host = provider_host();
        let route = &host.routes[0];
        let body = serde_json::json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "hello"}]
        });
        assert!((route.validate_request)(&serde_json::to_vec(&body).unwrap()).is_ok());
    }

    // --- /api/chat (Ollama native) ---

    #[test]
    fn chat_route_is_post_api_chat() {
        let host = provider_host();
        let route = &host.routes[1];
        assert_eq!(route.method, HttpMethod::Post);
        assert_eq!(route.path, "/api/chat");
    }

    #[test]
    fn chat_route_has_correct_backend_and_operation() {
        let host = provider_host();
        let route = &host.routes[1];
        assert_eq!(route.service_backend, ServiceBackend::Infer(InferenceBackendType::Ollama));
        assert_eq!(route.operation, Operation::Chat);
    }

    #[test]
    fn chat_route_rejects_invalid_request() {
        let host = provider_host();
        let route = &host.routes[1];
        assert!((route.validate_request)(b"not json").is_err());
    }

    #[test]
    fn chat_route_accepts_valid_request() {
        let host = provider_host();
        let route = &host.routes[1];
        let body = serde_json::json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "hello"}]
        });
        assert!((route.validate_request)(&serde_json::to_vec(&body).unwrap()).is_ok());
    }

    // --- /api/generate (Ollama native) ---

    #[test]
    fn generate_route_is_post_api_generate() {
        let host = provider_host();
        let route = &host.routes[2];
        assert_eq!(route.method, HttpMethod::Post);
        assert_eq!(route.path, "/api/generate");
    }

    #[test]
    fn generate_route_has_correct_backend_and_operation() {
        let host = provider_host();
        let route = &host.routes[2];
        assert_eq!(route.service_backend, ServiceBackend::Infer(InferenceBackendType::Ollama));
        assert_eq!(route.operation, Operation::Generate);
    }

    #[test]
    fn generate_route_rejects_invalid_request() {
        let host = provider_host();
        let route = &host.routes[2];
        assert!((route.validate_request)(b"not json").is_err());
    }

    #[test]
    fn generate_route_accepts_valid_request() {
        let host = provider_host();
        let route = &host.routes[2];
        let body = serde_json::json!({
            "model": "llama3.2",
            "prompt": "Why is the sky blue?"
        });
        assert!((route.validate_request)(&serde_json::to_vec(&body).unwrap()).is_ok());
    }

    #[test]
    fn generate_route_rejects_chat_request() {
        let host = provider_host();
        let route = &host.routes[2];
        // chat-shaped payload should fail: has messages, no prompt
        let body = serde_json::json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "hello"}]
        });
        assert!((route.validate_request)(&serde_json::to_vec(&body).unwrap()).is_err());
    }
}
