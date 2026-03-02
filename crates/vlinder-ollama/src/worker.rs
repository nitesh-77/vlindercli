//! Ollama worker — receives requests from the queue for inference
//! (Run, Chat, Generate) and embedding (Run on Embed service), POSTs
//! to the appropriate Ollama endpoint, and sends the response back.

use std::sync::Arc;
use std::time::Instant;

use async_openai::types::chat::{CreateChatCompletionRequest, CreateChatCompletionResponse};
use vlinder_core::domain::{
    EmbeddingBackendType, InferenceBackendType, MessageQueue, Operation, RequestMessage,
    ResponseMessage, ServiceBackend, ServiceDiagnostics, ServiceMetrics, ServiceType,
};

use crate::types::{
    OllamaChatRequest, OllamaChatResponse, OllamaEmbedRequest, OllamaEmbedResponse,
    OllamaGenerateRequest, OllamaGenerateResponse,
};

/// Successful inference result from the upstream API.
struct InferenceSuccess {
    body: Vec<u8>,
    tokens_input: u32,
    tokens_output: u32,
    model: String,
}

/// Successful embedding result from the upstream API.
struct EmbedSuccess {
    body: Vec<u8>,
    dimensions: u32,
    model: String,
}

/// Error from the upstream API or request validation.
struct WorkerError {
    status_code: u16,
    body: Vec<u8>,
}

pub struct OllamaWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    endpoint: String,
}

impl OllamaWorker {
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>, endpoint: String) -> Self {
        Self { queue, endpoint }
    }

    /// Process one message if available. Polls inference and embed queues.
    /// Returns true if a message was processed.
    pub fn tick(&self) -> bool {
        // Try each inference operation in turn.
        for op in [Operation::Run, Operation::Chat, Operation::Generate] {
            match self
                .queue
                .receive_request(ServiceBackend::Infer(InferenceBackendType::Ollama), op)
            {
                Ok((request, ack)) => {
                    self.process(request, ack, op);
                    return true;
                }
                Err(_) => continue,
            }
        }

        // Poll for embed requests.
        if let Ok((request, ack)) = self.queue.receive_request(
            ServiceBackend::Embed(EmbeddingBackendType::Ollama),
            Operation::Run,
        ) {
            self.process_embed(request, ack);
            return true;
        }

        false
    }

    fn process(
        &self,
        request: RequestMessage,
        ack: Box<dyn FnOnce() -> Result<(), vlinder_core::domain::QueueError> + Send>,
        operation: Operation,
    ) {
        let start = Instant::now();
        let payload = request.payload.as_slice();

        let (response_payload, status_code, tokens_input, tokens_output, model) = match operation {
            Operation::Run => match self.handle_openai(payload) {
                Ok(s) => (s.body, 200, s.tokens_input, s.tokens_output, s.model),
                Err(e) => (e.body, e.status_code, 0, 0, String::new()),
            },
            Operation::Chat => match self.handle_chat(payload) {
                Ok(s) => (s.body, 200, s.tokens_input, s.tokens_output, s.model),
                Err(e) => (e.body, e.status_code, 0, 0, String::new()),
            },
            Operation::Generate => match self.handle_generate(payload) {
                Ok(s) => (s.body, 200, s.tokens_input, s.tokens_output, s.model),
                Err(e) => (e.body, e.status_code, 0, 0, String::new()),
            },
            _ => (
                error_json("unsupported operation"),
                400,
                0,
                0,
                String::new(),
            ),
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        let diag = ServiceDiagnostics {
            service: ServiceType::Infer,
            backend: "ollama".to_string(),
            duration_ms,
            metrics: ServiceMetrics::Inference {
                tokens_input,
                tokens_output,
                model,
            },
        };

        let mut response =
            ResponseMessage::from_request_with_diagnostics(&request, response_payload, diag);
        response.state = request.state.clone();
        response.status_code = status_code;
        let _ = self.queue.send_response(response);
        let _ = ack();
    }

    // ---- OpenAI-compatible: /v1/chat/completions ----

    fn handle_openai(&self, payload: &[u8]) -> Result<InferenceSuccess, WorkerError> {
        let req: CreateChatCompletionRequest =
            serde_json::from_slice(payload).map_err(|e| WorkerError {
                status_code: 400,
                body: error_json(&e.to_string()),
            })?;

        let model_name = req.model.clone();

        let response = self
            .call_upstream("/v1/chat/completions", &req)
            .map_err(|e| WorkerError {
                status_code: 500,
                body: error_json(&e),
            })?;

        let resp: CreateChatCompletionResponse = response;
        let (ti, to) = resp
            .usage
            .as_ref()
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or((0, 0));

        let body = serde_json::to_vec(&resp).map_err(|e| WorkerError {
            status_code: 500,
            body: error_json(&e.to_string()),
        })?;

        Ok(InferenceSuccess {
            body,
            tokens_input: ti,
            tokens_output: to,
            model: model_name,
        })
    }

    // ---- Native: /api/chat ----

    fn handle_chat(&self, payload: &[u8]) -> Result<InferenceSuccess, WorkerError> {
        let req: OllamaChatRequest = serde_json::from_slice(payload).map_err(|e| WorkerError {
            status_code: 400,
            body: error_json(&e.to_string()),
        })?;

        let model_name = req.model.clone();

        let resp: OllamaChatResponse =
            self.call_upstream("/api/chat", &req)
                .map_err(|e| WorkerError {
                    status_code: 500,
                    body: error_json(&e),
                })?;

        let ti = resp.prompt_eval_count.unwrap_or(0);
        let to = resp.eval_count.unwrap_or(0);

        let body = serde_json::to_vec(&resp).map_err(|e| WorkerError {
            status_code: 500,
            body: error_json(&e.to_string()),
        })?;

        Ok(InferenceSuccess {
            body,
            tokens_input: ti,
            tokens_output: to,
            model: model_name,
        })
    }

    // ---- Native: /api/generate ----

    fn handle_generate(&self, payload: &[u8]) -> Result<InferenceSuccess, WorkerError> {
        let req: OllamaGenerateRequest =
            serde_json::from_slice(payload).map_err(|e| WorkerError {
                status_code: 400,
                body: error_json(&e.to_string()),
            })?;

        let model_name = req.model.clone();

        let resp: OllamaGenerateResponse =
            self.call_upstream("/api/generate", &req)
                .map_err(|e| WorkerError {
                    status_code: 500,
                    body: error_json(&e),
                })?;

        let ti = resp.prompt_eval_count.unwrap_or(0);
        let to = resp.eval_count.unwrap_or(0);

        let body = serde_json::to_vec(&resp).map_err(|e| WorkerError {
            status_code: 500,
            body: error_json(&e.to_string()),
        })?;

        Ok(InferenceSuccess {
            body,
            tokens_input: ti,
            tokens_output: to,
            model: model_name,
        })
    }

    // ---- Embed: /api/embed ----

    fn process_embed(
        &self,
        request: RequestMessage,
        ack: Box<dyn FnOnce() -> Result<(), vlinder_core::domain::QueueError> + Send>,
    ) {
        let start = Instant::now();
        let payload = request.payload.as_slice();

        let (response_payload, status_code, dimensions, model) = match self.handle_embed(payload) {
            Ok(s) => (s.body, 200, s.dimensions, s.model),
            Err(e) => (e.body, e.status_code, 0, String::new()),
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        let diag = ServiceDiagnostics {
            service: ServiceType::Embed,
            backend: "ollama".to_string(),
            duration_ms,
            metrics: ServiceMetrics::Embedding { dimensions, model },
        };

        let mut response =
            ResponseMessage::from_request_with_diagnostics(&request, response_payload, diag);
        response.state = request.state.clone();
        response.status_code = status_code;
        let _ = self.queue.send_response(response);
        let _ = ack();
    }

    fn handle_embed(&self, payload: &[u8]) -> Result<EmbedSuccess, WorkerError> {
        let req: OllamaEmbedRequest = serde_json::from_slice(payload).map_err(|e| WorkerError {
            status_code: 400,
            body: error_json(&e.to_string()),
        })?;

        let model_name = req.model.clone();

        let resp: OllamaEmbedResponse =
            self.call_upstream("/api/embed", &req)
                .map_err(|e| WorkerError {
                    status_code: 500,
                    body: error_json(&e),
                })?;

        let dimensions = resp.embeddings.first().map(|v| v.len() as u32).unwrap_or(0);

        let body = serde_json::to_vec(&resp).map_err(|e| WorkerError {
            status_code: 500,
            body: error_json(&e.to_string()),
        })?;

        Ok(EmbedSuccess {
            body,
            dimensions,
            model: model_name,
        })
    }

    // ---- HTTP ----

    fn call_upstream<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        req: &Req,
    ) -> Result<Resp, String> {
        let url = format!("{}{}", self.endpoint, path);

        let mut response = ureq::post(&url).send_json(req).map_err(|e| e.to_string())?;

        response
            .body_mut()
            .read_json()
            .map_err(|e| format!("failed to parse response: {}", e))
    }
}

/// Build an OpenAI-shaped error JSON payload.
fn error_json(message: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "error": {
            "message": message,
            "type": "server_error",
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
        AgentId, EmbeddingBackendType, InferenceBackendType, RequestDiagnostics, Sequence,
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

    fn send_infer_request(
        queue: &Arc<dyn MessageQueue + Send + Sync>,
        operation: Operation,
        payload: Vec<u8>,
        state: Option<String>,
    ) -> RequestMessage {
        let request = RequestMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-test".to_string()),
            SessionId::new(),
            AgentId::new("test-agent"),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
            operation,
            Sequence::first(),
            payload,
            state,
            test_request_diag(),
        );
        queue.send_request(request.clone()).unwrap();
        request
    }

    fn send_embed_request(
        queue: &Arc<dyn MessageQueue + Send + Sync>,
        payload: Vec<u8>,
        state: Option<String>,
    ) -> RequestMessage {
        let request = RequestMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-test".to_string()),
            SessionId::new(),
            AgentId::new("test-agent"),
            ServiceBackend::Embed(EmbeddingBackendType::Ollama),
            Operation::Run,
            Sequence::first(),
            payload,
            state,
            test_request_diag(),
        );
        queue.send_request(request.clone()).unwrap();
        request
    }

    fn make_worker(queue: &Arc<dyn MessageQueue + Send + Sync>) -> OllamaWorker {
        OllamaWorker::new(Arc::clone(queue), "http://127.0.0.1:1".to_string())
    }

    // --- Operation::Run (OpenAI-compatible) ---

    #[test]
    fn run_rejects_invalid_payload() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let request = send_infer_request(&queue, Operation::Run, b"not json".to_vec(), None);
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.status_code, 400);
        ack().unwrap();
    }

    #[test]
    fn run_echoes_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let body = serde_json::json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let request = send_infer_request(
            &queue,
            Operation::Run,
            serde_json::to_vec(&body).unwrap(),
            Some("xyz".to_string()),
        );
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.state, Some("xyz".to_string()));
        ack().unwrap();
    }

    #[test]
    fn run_unreachable_endpoint_returns_500() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let body = serde_json::json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let request = send_infer_request(
            &queue,
            Operation::Run,
            serde_json::to_vec(&body).unwrap(),
            None,
        );
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.status_code, 500);
        ack().unwrap();
    }

    // --- Operation::Chat (/api/chat) ---

    #[test]
    fn chat_rejects_invalid_payload() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let request = send_infer_request(&queue, Operation::Chat, b"not json".to_vec(), None);
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.status_code, 400);
        ack().unwrap();
    }

    #[test]
    fn chat_echoes_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let body = serde_json::json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let request = send_infer_request(
            &queue,
            Operation::Chat,
            serde_json::to_vec(&body).unwrap(),
            Some("abc".to_string()),
        );
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.state, Some("abc".to_string()));
        ack().unwrap();
    }

    #[test]
    fn chat_unreachable_endpoint_returns_500() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let body = serde_json::json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let request = send_infer_request(
            &queue,
            Operation::Chat,
            serde_json::to_vec(&body).unwrap(),
            None,
        );
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.status_code, 500);
        ack().unwrap();
    }

    // --- Operation::Generate (/api/generate) ---

    #[test]
    fn generate_rejects_invalid_payload() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let request = send_infer_request(&queue, Operation::Generate, b"not json".to_vec(), None);
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.status_code, 400);
        ack().unwrap();
    }

    #[test]
    fn generate_echoes_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let body = serde_json::json!({
            "model": "llama3.2",
            "prompt": "Why is the sky blue?"
        });
        let request = send_infer_request(
            &queue,
            Operation::Generate,
            serde_json::to_vec(&body).unwrap(),
            Some("def".to_string()),
        );
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.state, Some("def".to_string()));
        ack().unwrap();
    }

    #[test]
    fn generate_unreachable_endpoint_returns_500() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let body = serde_json::json!({
            "model": "llama3.2",
            "prompt": "Why is the sky blue?"
        });
        let request = send_infer_request(
            &queue,
            Operation::Generate,
            serde_json::to_vec(&body).unwrap(),
            None,
        );
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.status_code, 500);
        ack().unwrap();
    }

    // --- Embed (/api/embed) ---

    #[test]
    fn embed_rejects_invalid_payload() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let request = send_embed_request(&queue, b"not json".to_vec(), None);
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.status_code, 400);
        ack().unwrap();
    }

    #[test]
    fn embed_echoes_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let body = serde_json::json!({
            "model": "nomic-embed-text",
            "input": "hello world"
        });
        let request = send_embed_request(
            &queue,
            serde_json::to_vec(&body).unwrap(),
            Some("embed-state".to_string()),
        );
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.state, Some("embed-state".to_string()));
        ack().unwrap();
    }

    #[test]
    fn embed_unreachable_endpoint_returns_500() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);

        let body = serde_json::json!({
            "model": "nomic-embed-text",
            "input": "hello world"
        });
        let request = send_embed_request(&queue, serde_json::to_vec(&body).unwrap(), None);
        assert!(worker.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(response.status_code, 500);
        ack().unwrap();
    }

    // --- General ---

    #[test]
    fn no_message_returns_false() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let worker = make_worker(&queue);
        assert!(!worker.tick());
    }
}
