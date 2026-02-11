//! In-memory queue implementation.

use super::{
    CompleteMessage, DelegateMessage, InvokeMessage, MessageQueue,
    ObservableMessage, QueueError, RequestMessage, ResponseMessage, SubmissionId,
};
#[cfg(test)]
use super::{
    ContainerDiagnostics, DelegateDiagnostics, InvokeDiagnostics, RequestDiagnostics,
};
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

    fn receive_response(&self, request: &RequestMessage) -> Result<(ResponseMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let pattern = format!("{}.res.{}.{}", request.submission, request.service, request.backend);
        let mut typed = self.typed_queues.lock().unwrap();

        for (subject, queue) in typed.iter_mut() {
            if subject.contains(&pattern) {
                if let Some(ObservableMessage::Response(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();

                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn receive_complete(&self, submission: &SubmissionId, harness: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (subject, queue) in typed.iter_mut() {
            if subject.contains("complete") && subject.contains(submission.as_str()) && subject.contains(harness) {
                if let Some(ObservableMessage::Complete(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();

                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn send_delegate(&self, msg: DelegateMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.delegate.{}.{}",
            msg.submission, msg.caller_agent, msg.target_agent,
        );

        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(subject)
            .or_default()
            .push_back(ObservableMessage::Delegate(msg));
        Ok(())
    }

    fn receive_delegate(&self, target_agent: &str) -> Result<(DelegateMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (subject, queue) in typed.iter_mut() {
            if subject.contains("delegate") && subject.ends_with(target_agent) {
                if let Some(ObservableMessage::Delegate(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();

                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn send_complete_to_subject(&self, msg: CompleteMessage, subject: &str) -> Result<(), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(subject.to_string())
            .or_default()
            .push_back(ObservableMessage::Complete(msg));
        Ok(())
    }

    fn receive_complete_on_subject(&self, subject: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        if let Some(queue) = typed.get_mut(subject) {
            if let Some(ObservableMessage::Complete(msg)) = queue.front() {
                let msg = msg.clone();
                queue.pop_front();

                return Ok((msg, Box::new(|| Ok(()))));
            }
        }

        Err(QueueError::Timeout)
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

use super::agent_routing_key as agent_short_name;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ResourceId, RuntimeType};
    use crate::queue::{ExpectsReply, HarnessType, Sequence, SessionId, SubmissionId};

    fn test_agent_id() -> ResourceId {
        ResourceId::new("http://127.0.0.1:9000/agents/echo-agent")
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
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"input".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );

        queue.send_invoke(invoke).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        assert_eq!(typed.len(), 1);
        let (subject, messages) = typed.iter().next().unwrap();
        assert!(subject.contains("invoke"));
        assert!(subject.contains("cli"));
        assert!(subject.contains("container"));
        assert!(subject.contains("echo-agent"));
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn send_request_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
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
            SessionId::new(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::from(3),
            b"key".to_vec(),
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
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
            SessionId::new(),
            HarnessType::Web,
            RuntimeType::Container,
            test_agent_id(),
            b"input".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );
        let complete = invoke.create_reply(b"output".to_vec());

        queue.send_complete(complete).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        let (subject, _) = typed.iter().next().unwrap();

        assert_eq!(subject, "vlinder.sub-test-123.complete.echo-agent.web");
    }

    #[test]
    fn agent_short_name_extracts_from_registry_id() {
        let id = ResourceId::new("http://127.0.0.1:9000/agents/pensieve");
        assert_eq!(agent_short_name(&id), "pensieve");
    }

    #[test]
    fn agent_short_name_extracts_last_path_component() {
        let id = ResourceId::new("http://example.com/agents/test-agent");
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
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"hello".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );
        let original_id = invoke.id.clone();

        queue.send_invoke(invoke).unwrap();

        // Receive typed message
        let (received, ack) = queue.receive_invoke("invoke").unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.harness, HarnessType::Cli);
        assert_eq!(received.runtime, RuntimeType::Container);
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
            SessionId::new(),
            HarnessType::Web,
            RuntimeType::Container,
            agent_id.clone(),
            b"input".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
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
            SessionId::new(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
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
            SessionId::new(),
            agent_id.clone(),
            "vec",
            "sqlite-vec",
            "search",
            Sequence::from(3),
            b"query".to_vec(),
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
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
            SessionId::new(),
            test_agent_id(),
            "infer",
            "ollama",
            "",
            Sequence::first(),
            b"prompt".to_vec(),
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
        );

        queue.send_request(request).unwrap();

        // Receive with empty operation
        let (received, _) = queue.receive_request("infer", "ollama", "").unwrap();

        assert_eq!(received.service, "infer");
        assert_eq!(received.backend, "ollama");
        assert_eq!(received.operation, "");
    }

    // ========================================================================
    // Delegation tests (ADR 056)
    // ========================================================================

    #[test]
    fn send_delegate_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let delegate = DelegateMessage::new(
            test_submission(),
            SessionId::new(),
            "coordinator",
            "summarizer",
            b"summarize this".to_vec(),
            "vlinder.sub.reply.coordinator.summarizer.abc",
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );

        queue.send_delegate(delegate).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        assert_eq!(typed.len(), 1);
        let (subject, _) = typed.iter().next().unwrap();
        assert_eq!(subject, "vlinder.sub-test-123.delegate.coordinator.summarizer");
    }

    #[test]
    fn receive_delegate_returns_typed_message() {
        let queue = InMemoryQueue::new();

        let delegate = DelegateMessage::new(
            test_submission(),
            SessionId::new(),
            "coordinator",
            "summarizer",
            b"payload".to_vec(),
            "reply.subject",
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );
        let original_id = delegate.id.clone();

        queue.send_delegate(delegate).unwrap();

        let (received, ack) = queue.receive_delegate("summarizer").unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.caller_agent, "coordinator");
        assert_eq!(received.target_agent, "summarizer");
        assert_eq!(received.payload, b"payload");
        assert_eq!(received.reply_subject, "reply.subject");

        ack().unwrap();
    }

    #[test]
    fn receive_delegate_times_out_for_wrong_target() {
        let queue = InMemoryQueue::new();

        let delegate = DelegateMessage::new(
            test_submission(),
            SessionId::new(),
            "coordinator",
            "summarizer",
            b"payload".to_vec(),
            "reply.subject",
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );

        queue.send_delegate(delegate).unwrap();

        let result = queue.receive_delegate("fact-checker");
        assert!(matches!(result, Err(QueueError::Timeout)));
    }

    #[test]
    fn send_and_receive_complete_on_subject() {
        let queue = InMemoryQueue::new();

        let complete = CompleteMessage::new(
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            HarnessType::Cli,
            b"result".to_vec(),
            None,
            ContainerDiagnostics::placeholder(0),
        );

        let subject = "vlinder.sub.delegate-reply.coordinator.summarizer.abc123";
        queue.send_complete_to_subject(complete, subject).unwrap();

        let (received, ack) = queue.receive_complete_on_subject(subject).unwrap();
        assert_eq!(received.payload, b"result");
        ack().unwrap();
    }

    #[test]
    fn receive_complete_on_subject_times_out_for_wrong_subject() {
        let queue = InMemoryQueue::new();

        let complete = CompleteMessage::new(
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            HarnessType::Cli,
            b"result".to_vec(),
            None,
            ContainerDiagnostics::placeholder(0),
        );

        queue.send_complete_to_subject(complete, "subject.a").unwrap();

        let result = queue.receive_complete_on_subject("subject.b");
        assert!(matches!(result, Err(QueueError::Timeout)));
    }
}
