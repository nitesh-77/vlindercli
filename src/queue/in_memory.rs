//! In-memory queue implementation.

use crate::domain::{
    CompleteMessage, DelegateMessage, InvokeMessage, MessageQueue,
    ObservableMessage, Operation, QueueError, RequestMessage, ResponseMessage,
    RoutingKey, ServiceType, SubmissionId,
};
#[cfg(test)]
use crate::domain::{
    AgentId, ContainerDiagnostics, DelegateDiagnostics, InvokeDiagnostics, RequestDiagnostics,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// In-memory message queue for single-process use.
///
/// Messages are keyed by `RoutingKey` (ADR 096) — collision-freedom is
/// structural, not dependent on string formatting. Delegate reply messages
/// use a separate string-keyed map (pending §7 elimination).
///
/// ACK/NACK operations are no-ops since messages are removed from the queue
/// immediately on receive (no durability or redelivery support).
pub struct InMemoryQueue {
    /// Messages keyed by RoutingKey (ADR 096 §5).
    pub(crate) typed_queues: Arc<Mutex<HashMap<RoutingKey, VecDeque<ObservableMessage>>>>,
    /// Delegate reply messages keyed by opaque subject string.
    /// Temporary — will be replaced by RoutingKey::DelegateReply in §7.
    reply_queues: Arc<Mutex<HashMap<String, VecDeque<ObservableMessage>>>>,
}

impl InMemoryQueue {
    pub fn new() -> Self {
        Self {
            typed_queues: Arc::new(Mutex::new(HashMap::new())),
            reply_queues: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageQueue for InMemoryQueue {
    fn service_queue(&self, service: ServiceType, backend: &str, action: Operation) -> String {
        format!("vlinder.svc.{}.{}.{}", service, backend, action)
    }

    fn agent_queue(&self, runtime: &str, agent: &crate::domain::Agent) -> String {
        format!("vlinder.agent.{}.{}", runtime, agent.name)
    }

    fn send_invoke(&self, msg: InvokeMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed.entry(key).or_default().push_back(ObservableMessage::Invoke(msg));
        Ok(())
    }

    fn send_request(&self, msg: RequestMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed.entry(key).or_default().push_back(ObservableMessage::Request(msg));
        Ok(())
    }

    fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed.entry(key).or_default().push_back(ObservableMessage::Response(msg));
        Ok(())
    }

    fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed.entry(key).or_default().push_back(ObservableMessage::Complete(msg));
        Ok(())
    }

    fn receive_invoke(&self, subject_pattern: &str) -> Result<(InvokeMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey::Invoke { agent, .. } => {
                    subject_pattern.contains('*') || agent.as_str() == subject_pattern
                }
                _ => false,
            };
            if matches {
                if let Some(ObservableMessage::Invoke(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();
                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn receive_request(&self, service: ServiceType, backend: &str, operation: Operation) -> Result<(RequestMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey::Request { service: svc, operation: op, .. } => {
                    svc.service_type() == service && svc.backend_str() == backend && *op == operation
                }
                _ => false,
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
        // Exact lookup via reply_key (ADR 096 §6).
        let reply_key = request.routing_key().reply_key(None).unwrap();
        let mut typed = self.typed_queues.lock().unwrap();

        if let Some(queue) = typed.get_mut(&reply_key) {
            if let Some(ObservableMessage::Response(msg)) = queue.front() {
                let msg = msg.clone();
                queue.pop_front();
                return Ok((msg, Box::new(|| Ok(()))));
            }
        }

        Err(QueueError::Timeout)
    }

    fn receive_complete(&self, submission: &SubmissionId, harness: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey::Complete { submission: sub, harness: h, .. } => {
                    sub == submission && h.as_str() == harness
                }
                _ => false,
            };
            if matches {
                if let Some(ObservableMessage::Complete(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();
                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn create_reply_address(&self, submission: &SubmissionId, caller: &str, target: &str) -> String {
        let short_uuid = &uuid::Uuid::new_v4().to_string()[..8];
        format!("reply.{}.{}.{}.{}", submission, caller, target, short_uuid)
    }

    fn send_delegate(&self, msg: DelegateMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed.entry(key).or_default().push_back(ObservableMessage::Delegate(msg));
        Ok(())
    }

    fn receive_delegate(&self, target_agent: &str) -> Result<(DelegateMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey::Delegate { target, .. } => target.as_str() == target_agent,
                _ => false,
            };
            if matches {
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
        let mut replies = self.reply_queues.lock().unwrap();
        replies
            .entry(subject.to_string())
            .or_default()
            .push_back(ObservableMessage::Complete(msg));
        Ok(())
    }

    fn receive_complete_on_subject(&self, subject: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut replies = self.reply_queues.lock().unwrap();

        if let Some(queue) = replies.get_mut(subject) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{InferenceBackendType, ObjectStorageType, Operation, RuntimeType, ServiceBackend, VectorStorageType};
    use crate::domain::{HarnessType, Sequence, SessionId, SubmissionId, TimelineId};

    fn test_agent_id() -> AgentId {
        AgentId::new("echo-agent")
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-123".to_string())
    }

    // ========================================================================
    // Typed receive tests
    // ========================================================================

    #[test]
    fn receive_invoke_returns_typed_message() {
        let queue = InMemoryQueue::new();

        let invoke = InvokeMessage::new(
            TimelineId::main(),
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
        let (received, ack) = queue.receive_invoke("echo-agent").unwrap();

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
            TimelineId::main(),
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

        let (received, _) = queue.receive_invoke("echo-agent").unwrap();

        // All dimensions preserved for reply construction
        assert_eq!(received.submission, submission);
        assert_eq!(received.agent_id, agent_id);
        assert_eq!(received.harness, HarnessType::Web);
    }

    #[test]
    fn receive_request_returns_typed_message() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::first(),
            b"key".to_vec(),
            None,
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
        );
        let original_id = request.id.clone();

        queue.send_request(request).unwrap();

        // Receive by service/backend/operation
        let (received, ack) = queue.receive_request(ServiceType::Kv, "sqlite", Operation::Get).unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.service, ServiceBackend::Kv(ObjectStorageType::Sqlite));
        assert_eq!(received.operation, Operation::Get);
        assert_eq!(received.payload.legacy_bytes(), b"key");

        ack().unwrap();
    }

    #[test]
    fn receive_request_preserves_all_dimensions() {
        let queue = InMemoryQueue::new();

        let submission = test_submission();
        let agent_id = test_agent_id();

        let request = RequestMessage::new(
            TimelineId::main(),
            submission.clone(),
            SessionId::new(),
            agent_id.clone(),
            ServiceBackend::Vec(VectorStorageType::SqliteVec),
            Operation::Search,
            Sequence::from(3),
            b"query".to_vec(),
            None,
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
        );

        queue.send_request(request).unwrap();

        let (received, _) = queue.receive_request(ServiceType::Vec, "sqlite-vec", Operation::Search).unwrap();

        // All dimensions preserved for reply construction
        assert_eq!(received.submission, submission);
        assert_eq!(received.agent_id, agent_id);
        assert_eq!(received.sequence, Sequence::from(3));
    }

    #[test]
    fn receive_request_matches_infer_run() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
            Operation::Run,
            Sequence::first(),
            b"prompt".to_vec(),
            None,
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
        );

        queue.send_request(request).unwrap();

        let (received, _) = queue.receive_request(ServiceType::Infer, "ollama", Operation::Run).unwrap();

        assert_eq!(received.service, ServiceBackend::Infer(InferenceBackendType::Ollama));
        assert_eq!(received.operation, Operation::Run);
    }

    // ========================================================================
    // Delegation tests (ADR 056)
    // ========================================================================

    #[test]
    fn create_reply_address_is_unique() {
        let queue = InMemoryQueue::new();
        let submission = test_submission();

        let addr1 = queue.create_reply_address(&submission, "caller", "target");
        let addr2 = queue.create_reply_address(&submission, "caller", "target");

        assert!(!addr1.is_empty());
        assert!(!addr2.is_empty());
        assert_ne!(addr1, addr2, "each reply address must be unique");
        assert!(addr1.contains("caller"));
        assert!(addr1.contains("target"));
        assert!(addr1.contains(submission.as_str()));
    }

    #[test]
    fn receive_delegate_returns_typed_message() {
        let queue = InMemoryQueue::new();

        let delegate = DelegateMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"payload".to_vec(),
            "reply.subject",
            None,
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );
        let original_id = delegate.id.clone();

        queue.send_delegate(delegate).unwrap();

        let (received, ack) = queue.receive_delegate("summarizer").unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.caller, AgentId::new("coordinator"));
        assert_eq!(received.target, AgentId::new("summarizer"));
        assert_eq!(received.payload, b"payload");
        assert_eq!(received.reply_subject, "reply.subject");

        ack().unwrap();
    }

    #[test]
    fn receive_delegate_times_out_for_wrong_target() {
        let queue = InMemoryQueue::new();

        let delegate = DelegateMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"payload".to_vec(),
            "reply.subject",
            None,
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
            TimelineId::main(),
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
            TimelineId::main(),
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
