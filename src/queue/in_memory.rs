//! In-memory queue implementation.

use super::{
    CompleteMessage, InvokeMessage, MessageQueue, ObservableMessage,
    QueueError, RequestMessage, ResponseMessage,
};
use crate::domain::ResourceId;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// In-memory message queue for single-process use.
///
/// ACK/NACK operations are no-ops since messages are removed from the queue
/// immediately on receive (no durability or redelivery support).
///
/// Uses typed messages exclusively (ADR 044).
pub struct InMemoryQueue {
    /// Typed queues keyed by subject
    pub(crate) typed_queues: Arc<Mutex<HashMap<String, VecDeque<ObservableMessage>>>>,
}

impl InMemoryQueue {
    pub fn new() -> Self {
        Self {
            typed_queues: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageQueue for InMemoryQueue {
    fn service_queue(&self, service: &str, backend: &str, action: &str) -> String {
        if action.is_empty() {
            format!("vlinder.svc.{}.{}", service, backend)
        } else {
            format!("vlinder.svc.{}.{}.{}", service, backend, action)
        }
    }

    fn agent_queue(&self, runtime: &str, agent: &crate::domain::Agent) -> String {
        format!("vlinder.agent.{}.{}", runtime, agent.name)
    }

    fn send_invoke(&self, msg: InvokeMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.invoke.{}.{}.{}",
            msg.submission,
            msg.harness,
            msg.runtime.as_str(),
            agent_short_name(&msg.agent_id),
        );


        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(subject)
            .or_default()
            .push_back(ObservableMessage::Invoke(msg));
        Ok(())
    }

    fn send_request(&self, msg: RequestMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.req.{}.{}.{}.{}.{}",
            msg.submission,
            agent_short_name(&msg.agent_id),
            msg.service,
            msg.backend,
            msg.operation,
            msg.sequence,
        );


        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(subject)
            .or_default()
            .push_back(ObservableMessage::Request(msg));
        Ok(())
    }

    fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.res.{}.{}.{}.{}.{}",
            msg.submission,
            msg.service,
            msg.backend,
            agent_short_name(&msg.agent_id),
            msg.operation,
            msg.sequence,
        );


        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(subject)
            .or_default()
            .push_back(ObservableMessage::Response(msg));
        Ok(())
    }

    fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.complete.{}.{}",
            msg.submission,
            agent_short_name(&msg.agent_id),
            msg.harness,
        );


        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(subject)
            .or_default()
            .push_back(ObservableMessage::Complete(msg));
        Ok(())
    }

    fn receive_invoke(&self, subject_pattern: &str) -> Result<(InvokeMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (subject, queue) in typed.iter_mut() {
            if subject.contains(subject_pattern) || subject_pattern.contains("*") {
                if let Some(ObservableMessage::Invoke(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();

                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn receive_request(&self, service: &str, backend: &str, operation: &str) -> Result<(RequestMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        let pattern = format!(".{}.{}.{}.", service, backend, operation);
        let bare_pattern = format!(".{}.{}.", service, backend);

        for (subject, queue) in typed.iter_mut() {
            let matches = if operation.is_empty() {
                subject.contains(&bare_pattern) && subject.contains(".req.")
            } else {
                subject.contains(&pattern) && subject.contains(".req.")
            };

            if matches {
                if let Some(ObservableMessage::Request(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();

                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn receive_response(&self, subject_pattern: &str) -> Result<(ResponseMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (subject, queue) in typed.iter_mut() {
            if subject.contains(subject_pattern) || subject_pattern.contains("*") {
                if let Some(ObservableMessage::Response(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();

                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn receive_complete(&self, harness_pattern: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (subject, queue) in typed.iter_mut() {
            if subject.contains("complete") && subject.contains(harness_pattern) {
                if let Some(ObservableMessage::Complete(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();

                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Extract a short name from a ResourceId for queue routing.
///
/// - `file:///path/to/echo-agent.wasm` → "echo-agent"
/// - `memory://test-agent` → "test-agent"
fn agent_short_name(agent_id: &ResourceId) -> String {
    if let Some(path) = agent_id.path() {
        if let Some(filename) = path.rsplit('/').next() {
            let name = filename.strip_suffix(".wasm").unwrap_or(filename);
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    if let Some(authority) = agent_id.authority() {
        return authority.to_string();
    }
    agent_id.as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RuntimeType;
    use crate::queue::{ExpectsReply, HarnessType, Sequence, SubmissionId};

    fn test_agent_id() -> ResourceId {
        ResourceId::new("file:///test/echo-agent.wasm")
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-123".to_string())
    }

    // ========================================================================
    // Subject building tests
    // ========================================================================

    #[test]
    fn send_invoke_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let invoke = InvokeMessage::new(
            test_submission(),
            HarnessType::Cli,
            RuntimeType::Wasm,
            test_agent_id(),
            b"input".to_vec(),
        );

        queue.send_invoke(invoke).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        assert_eq!(typed.len(), 1);
        let (subject, messages) = typed.iter().next().unwrap();
        assert!(subject.contains("invoke"));
        assert!(subject.contains("cli"));
        assert!(subject.contains("wasm"));
        assert!(subject.contains("echo-agent"));
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn send_request_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            test_submission(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
        );

        queue.send_request(request).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        let (subject, _) = typed.iter().next().unwrap();

        // Subject follows ADR 044 pattern
        assert_eq!(subject, "vlinder.sub-test-123.req.echo-agent.kv.sqlite.get.1");
    }

    #[test]
    fn send_response_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            test_submission(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::from(3),
            b"key".to_vec(),
        );
        let response = request.create_reply(b"value".to_vec());

        queue.send_response(response).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        let (subject, _) = typed.iter().next().unwrap();

        // Response subject has service.backend before agent (per ADR 044)
        assert_eq!(subject, "vlinder.sub-test-123.res.kv.sqlite.echo-agent.get.3");
    }

    #[test]
    fn send_complete_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let invoke = InvokeMessage::new(
            test_submission(),
            HarnessType::Web,
            RuntimeType::Wasm,
            test_agent_id(),
            b"input".to_vec(),
        );
        let complete = invoke.create_reply(b"output".to_vec());

        queue.send_complete(complete).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        let (subject, _) = typed.iter().next().unwrap();

        assert_eq!(subject, "vlinder.sub-test-123.complete.echo-agent.web");
    }

    #[test]
    fn agent_short_name_extracts_from_file_uri() {
        let id = ResourceId::new("file:///path/to/pensieve.wasm");
        assert_eq!(agent_short_name(&id), "pensieve");
    }

    #[test]
    fn agent_short_name_extracts_from_memory_uri() {
        let id = ResourceId::new("memory://test-agent");
        assert_eq!(agent_short_name(&id), "test-agent");
    }

    // ========================================================================
    // Typed receive tests
    // ========================================================================

    #[test]
    fn receive_invoke_returns_typed_message() {
        let queue = InMemoryQueue::new();

        let invoke = InvokeMessage::new(
            test_submission(),
            HarnessType::Cli,
            RuntimeType::Wasm,
            test_agent_id(),
            b"hello".to_vec(),
        );
        let original_id = invoke.id.clone();

        queue.send_invoke(invoke).unwrap();

        // Receive typed message
        let (received, ack) = queue.receive_invoke("invoke").unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.harness, HarnessType::Cli);
        assert_eq!(received.runtime, RuntimeType::Wasm);
        assert_eq!(received.payload, b"hello");

        ack().unwrap();
    }

    #[test]
    fn receive_invoke_preserves_all_dimensions() {
        let queue = InMemoryQueue::new();

        let submission = test_submission();
        let agent_id = test_agent_id();

        let invoke = InvokeMessage::new(
            submission.clone(),
            HarnessType::Web,
            RuntimeType::Wasm,
            agent_id.clone(),
            b"input".to_vec(),
        );

        queue.send_invoke(invoke).unwrap();

        let (received, _) = queue.receive_invoke("invoke").unwrap();

        // All dimensions preserved for reply construction
        assert_eq!(received.submission, submission);
        assert_eq!(received.agent_id, agent_id);
        assert_eq!(received.harness, HarnessType::Web);
    }

    #[test]
    fn receive_request_returns_typed_message() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            test_submission(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
        );
        let original_id = request.id.clone();

        queue.send_request(request).unwrap();

        // Receive by service/backend/operation
        let (received, ack) = queue.receive_request("kv", "sqlite", "get").unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.service, "kv");
        assert_eq!(received.backend, "sqlite");
        assert_eq!(received.operation, "get");
        assert_eq!(received.payload, b"key");

        ack().unwrap();
    }

    #[test]
    fn receive_request_preserves_all_dimensions() {
        let queue = InMemoryQueue::new();

        let submission = test_submission();
        let agent_id = test_agent_id();

        let request = RequestMessage::new(
            submission.clone(),
            agent_id.clone(),
            "vec",
            "sqlite-vec",
            "search",
            Sequence::from(3),
            b"query".to_vec(),
        );

        queue.send_request(request).unwrap();

        let (received, _) = queue.receive_request("vec", "sqlite-vec", "search").unwrap();

        // All dimensions preserved for reply construction
        assert_eq!(received.submission, submission);
        assert_eq!(received.agent_id, agent_id);
        assert_eq!(received.sequence, Sequence::from(3));
    }

    #[test]
    fn receive_request_matches_bare_service() {
        let queue = InMemoryQueue::new();

        // For inference, operation is empty
        let request = RequestMessage::new(
            test_submission(),
            test_agent_id(),
            "infer",
            "ollama",
            "",
            Sequence::first(),
            b"prompt".to_vec(),
        );

        queue.send_request(request).unwrap();

        // Receive with empty operation
        let (received, _) = queue.receive_request("infer", "ollama", "").unwrap();

        assert_eq!(received.service, "infer");
        assert_eq!(received.backend, "ollama");
        assert_eq!(received.operation, "");
    }
}
