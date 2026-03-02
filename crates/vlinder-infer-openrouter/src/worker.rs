//! OpenRouter inference worker — receives OpenAI-typed requests from the queue,
//! POSTs to the OpenRouter API, and sends the full response back.

use std::sync::Arc;
use std::time::Instant;

use async_openai::types::chat::{CreateChatCompletionRequest, CreateChatCompletionResponse};
use vlinder_core::domain::{
    InferenceBackendType, MessageQueue, Operation, ResponseMessage, ServiceBackend,
    ServiceDiagnostics, ServiceMetrics, ServiceType,
};

/// Successful inference result from the upstream API.
struct InferenceSuccess {
    body: Vec<u8>,
    tokens_input: u32,
    tokens_output: u32,
    model: String,
}

/// Error from the upstream API or request validation.
struct WorkerError {
    status_code: u16,
    body: Vec<u8>,
}

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

                let (response_payload, status_code, tokens_input, tokens_output, model) =
                    match self.handle(request.payload.as_slice()) {
                        Ok(s) => (s.body, 200, s.tokens_input, s.tokens_output, s.model),
                        Err(e) => (e.body, e.status_code, 0, 0, String::new()),
                    };

                let duration_ms = start.elapsed().as_millis() as u64;

                let diag = ServiceDiagnostics {
                    service: ServiceType::Infer,
                    backend: "openrouter".to_string(),
                    duration_ms,
                    metrics: ServiceMetrics::Inference {
                        tokens_input,
                        tokens_output,
                        model,
                    },
                };

                let mut response = ResponseMessage::from_request_with_diagnostics(
                    &request,
                    response_payload,
                    diag,
                );
                response.state = request.state.clone();
                response.status_code = status_code;
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn handle(&self, payload: &[u8]) -> Result<InferenceSuccess, WorkerError> {
        let req: CreateChatCompletionRequest =
            serde_json::from_slice(payload).map_err(|e| WorkerError {
                status_code: 400,
                body: openai_error_json(&e.to_string(), "invalid_request_error"),
            })?;

        let model_name = req.model.clone();

        let response = self.call_openrouter(&req).map_err(|e| WorkerError {
            status_code: 500,
            body: openai_error_json(&e, "server_error"),
        })?;

        let (tokens_input, tokens_output) = response
            .usage
            .as_ref()
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or((0, 0));

        let body = serde_json::to_vec(&response).map_err(|e| WorkerError {
            status_code: 500,
            body: openai_error_json(&e.to_string(), "server_error"),
        })?;

        Ok(InferenceSuccess {
            body,
            tokens_input,
            tokens_output,
            model: model_name,
        })
    }

    fn call_openrouter(
        &self,
        req: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, String> {
        let url = format!("{}/chat/completions", self.endpoint);

        let mut response = ureq::post(&url)
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(req)
            .map_err(|e| e.to_string())?;

        response
            .body_mut()
            .read_json()
            .map_err(|e| format!("failed to parse response: {}", e))
    }
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
        AgentId, InferenceBackendType, RequestDiagnostics, RequestMessage, Sequence,
        ServiceBackend, SessionId, SubmissionId, TimelineId,
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
            TimelineId::main(),
            SubmissionId::from("sub-test".to_string()),
            SessionId::new(),
            AgentId::new("test-agent"),
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
        assert_eq!(response.status_code, 400);
        let body: serde_json::Value = serde_json::from_slice(response.payload.as_slice()).unwrap();
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
        assert_eq!(response.status_code, 500);
        let body: serde_json::Value = serde_json::from_slice(response.payload.as_slice()).unwrap();
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
