//! `OpenRouter` inference worker — receives OpenAI-typed requests from the queue,
//! POSTs to the `OpenRouter` API, and sends the full response back.

use std::sync::Arc;
use std::time::Instant;

use async_openai::types::chat::{CreateChatCompletionRequest, CreateChatCompletionResponse};
use vlinder_core::domain::{
    InferenceBackendType, MessageQueue, Operation, ResponseMessage, ServiceBackend,
    ServiceDiagnostics, ServiceMetrics, ServiceType,
};

/// Handler result: the raw HTTP response + extracted metrics.
type HandlerResult = (http::Response<Vec<u8>>, ServiceMetrics);

pub struct OpenRouterWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    endpoint: String,
    api_key: String,
}

impl OpenRouterWorker {
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        endpoint: String,
        api_key: String,
    ) -> Self {
        Self {
            queue,
            endpoint,
            api_key,
        }
    }

    /// Process one message if available. Returns true if a message was processed.
    pub fn tick(&self) -> bool {
        match self.queue.receive_request(
            ServiceBackend::Infer(InferenceBackendType::OpenRouter),
            Operation::Run,
        ) {
            Ok((request, ack)) => {
                let start = Instant::now();

                let (http_response, metrics) = self.handle(request.payload.as_slice());

                let status_code = http_response.status().as_u16();
                let wire = vlinder_core::domain::wire::WireResponse {
                    inner: http_response,
                };
                let response_payload = serde_json::to_vec(&wire).unwrap_or_default();

                let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

                let diag = ServiceDiagnostics {
                    service: ServiceType::Infer,
                    backend: "openrouter".to_string(),
                    duration_ms,
                    metrics,
                };

                let mut response = ResponseMessage::from_request_with_diagnostics(
                    &request,
                    response_payload,
                    diag,
                );
                response.state.clone_from(&request.state);
                response.status_code = status_code;
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn handle(&self, payload: &[u8]) -> HandlerResult {
        let req: CreateChatCompletionRequest = match serde_json::from_slice(payload) {
            Ok(r) => r,
            Err(e) => return error_result(400, &e.to_string(), "invalid_request_error"),
        };

        let model_name = req.model.clone();

        let http_response = match self.call_upstream_raw(&req) {
            Ok(r) => r,
            Err(e) => return error_result(500, &e, "server_error"),
        };

        let (ti, to) = serde_json::from_slice::<CreateChatCompletionResponse>(http_response.body())
            .ok()
            .and_then(|r| {
                r.usage
                    .as_ref()
                    .map(|u| (u.prompt_tokens, u.completion_tokens))
            })
            .unwrap_or((0, 0));

        (
            http_response,
            ServiceMetrics::Inference {
                tokens_input: ti,
                tokens_output: to,
                model: model_name,
            },
        )
    }

    fn call_upstream_raw(
        &self,
        req: &CreateChatCompletionRequest,
    ) -> Result<http::Response<Vec<u8>>, String> {
        let url = format!("{}/chat/completions", self.endpoint);

        let mut response = ureq::post(&url)
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(req)
            .map_err(|e| e.to_string())?;

        let status = response.status();
        let mut builder = http::Response::builder().status(status);
        for name in response.headers().keys() {
            for value in response.headers().get_all(name) {
                if let Ok(s) = value.to_str() {
                    builder = builder.header(name.as_str(), s);
                }
            }
        }

        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| format!("failed to read response body: {e}"))?;

        builder
            .body(body)
            .map_err(|e| format!("failed to build http::Response: {e}"))
    }
}

/// Build an error result with an OpenAI-shaped error body and zeroed metrics.
fn error_result(status: u16, message: &str, error_type: &str) -> HandlerResult {
    (
        http::Response::builder()
            .status(status)
            .body(openai_error_json(message, error_type))
            .expect("building error response with known-valid status"),
        ServiceMetrics::Inference {
            tokens_input: 0,
            tokens_output: 0,
            model: String::new(),
        },
    )
}

/// Build an OpenAI-shaped error JSON payload.
fn openai_error_json(message: &str, error_type: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "error": {
            "message": message,
            "type": error_type,
            "param": null,
            "code": null
        }
    }))
    .expect("JSON serialization cannot fail for static shape")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vlinder_core::domain::{
        AgentName, BranchId, InferenceBackendType, RequestDiagnostics, RequestMessage, Sequence,
        ServiceBackend, SessionId, SubmissionId,
    };
    use vlinder_core::queue::InMemoryQueue;

    fn test_request_diag() -> RequestDiagnostics {
        RequestDiagnostics {
            sequence: 0,
            endpoint: String::new(),
            request_bytes: 0,
            received_at_ms: 0,
        }
    }

    fn send_request(
        queue: &Arc<dyn MessageQueue + Send + Sync>,
        payload: Vec<u8>,
        state: Option<String>,
    ) -> RequestMessage {
        let request = RequestMessage::new(
            BranchId::from(1),
            SubmissionId::from("sub-test".to_string()),
            SessionId::new(),
            AgentName::new("test-agent"),
            ServiceBackend::Infer(InferenceBackendType::OpenRouter),
            Operation::Run,
            Sequence::first(),
            payload,
            state,
            test_request_diag(),
        );
        queue.send_request(request.clone()).unwrap();
        request
    }

    /// Unwrap the `WireResponse` envelope from a response payload, returning
    /// the HTTP status and body for assertions.
    fn unwrap_wire(response: &ResponseMessage) -> (u16, Vec<u8>) {
        if let Ok(wire) =
            serde_json::from_slice::<vlinder_core::domain::wire::WireResponse>(&response.payload)
        {
            (wire.inner.status().as_u16(), wire.inner.into_body())
        } else {
            (response.status_code, response.payload.clone())
        }
    }

    #[test]
    fn rejects_invalid_payload() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = OpenRouterWorker::new(
            Arc::clone(&queue),
            "http://127.0.0.1:1".to_string(),
            "test-key".to_string(),
        );

        let request = send_request(&queue, b"not json".to_vec(), None);
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        let (status, body) = unwrap_wire(&response);
        assert_eq!(status, 400);
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert!(!body["error"]["message"].as_str().unwrap().is_empty());
        ack().unwrap();
    }

    #[test]
    fn echoes_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = OpenRouterWorker::new(
            Arc::clone(&queue),
            "http://127.0.0.1:1".to_string(),
            "test-key".to_string(),
        );

        let body = serde_json::json!({
            "model": "anthropic/claude-sonnet-4",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let request = send_request(
            &queue,
            serde_json::to_vec(&body).unwrap(),
            Some("xyz".to_string()),
        );
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(
            response.state,
            Some("xyz".to_string()),
            "response should echo request state"
        );
        ack().unwrap();
    }

    #[test]
    fn valid_request_to_unreachable_endpoint_returns_error() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = OpenRouterWorker::new(
            Arc::clone(&queue),
            "http://127.0.0.1:1".to_string(),
            "test-key".to_string(),
        );

        let body = serde_json::json!({
            "model": "anthropic/claude-sonnet-4",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let request = send_request(&queue, serde_json::to_vec(&body).unwrap(), None);
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        let (status, body) = unwrap_wire(&response);
        assert_eq!(status, 500);
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["type"], "server_error");
        assert!(!body["error"]["message"].as_str().unwrap().is_empty());
        ack().unwrap();
    }

    #[test]
    fn no_message_returns_false() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = OpenRouterWorker::new(
            Arc::clone(&queue),
            "http://127.0.0.1:1".to_string(),
            "test-key".to_string(),
        );

        assert!(
            !worker.tick(),
            "tick() should return false when queue is empty"
        );
    }
}
