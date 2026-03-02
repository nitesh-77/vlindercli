//! In-memory queue implementation.

use crate::domain::{
    Acknowledgement, AgentId, CompleteMessage, DelegateMessage, HarnessType, InvokeMessage,
    MessageQueue, ObservableMessage, Operation, QueueError, RequestMessage, ResponseMessage,
    RoutingKey, ServiceBackend, SubmissionId,
};
#[cfg(test)]
use crate::domain::{
    DelegateDiagnostics, InvokeDiagnostics, RequestDiagnostics, RuntimeDiagnostics,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// In-memory message queue for single-process use.
///
/// Messages are keyed by `RoutingKey` (ADR 096) — collision-freedom is
/// structural, not dependent on string formatting.
///
/// ACK/NACK operations are no-ops since messages are removed from the queue
/// immediately on receive (no durability or redelivery support).
pub struct InMemoryQueue {
    /// Messages keyed by RoutingKey (ADR 096 §5).
    pub(crate) typed_queues: Arc<Mutex<HashMap<RoutingKey, VecDeque<ObservableMessage>>>>,
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
    fn send_invoke(&self, msg: InvokeMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(key)
            .or_default()
            .push_back(ObservableMessage::Invoke(msg));
        Ok(())
    }

    fn send_request(&self, msg: RequestMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(key)
            .or_default()
            .push_back(ObservableMessage::Request(msg));
        Ok(())
    }

    fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(key)
            .or_default()
            .push_back(ObservableMessage::Response(msg));
        Ok(())
    }

    fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(key)
            .or_default()
            .push_back(ObservableMessage::Complete(msg));
        Ok(())
    }

    fn receive_invoke(
        &self,
        agent: &AgentId,
    ) -> Result<(InvokeMessage, Acknowledgement), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey::Invoke { agent: a, .. } => a == agent,
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

    fn receive_request(
        &self,
        service: ServiceBackend,
        operation: Operation,
    ) -> Result<(RequestMessage, Acknowledgement), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey::Request {
                    service: svc,
                    operation: op,
                    ..
                } => *svc == service && *op == operation,
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

    fn receive_response(
        &self,
        request: &RequestMessage,
    ) -> Result<
        (
            ResponseMessage,
            Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
        ),
        QueueError,
    > {
        // Exact lookup via reply_key (ADR 096 §6).
        let reply_key = request.routing_key().reply_key(None).unwrap();
        let mut typed = self.typed_queues.lock().unwrap();

        if let Some(queue) = typed.get_mut(&reply_key) {
            if let Some(ObservableMessage::Response(_)) = queue.front() {
                if let Some(ObservableMessage::Response(msg)) = queue.pop_front() {
                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn receive_complete(
        &self,
        submission: &SubmissionId,
        harness: HarnessType,
    ) -> Result<
        (
            CompleteMessage,
            Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
        ),
        QueueError,
    > {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey::Complete {
                    submission: sub,
                    harness: h,
                    ..
                } => sub == submission && *h == harness,
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

    fn send_delegate(&self, msg: DelegateMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(key)
            .or_default()
            .push_back(ObservableMessage::Delegate(msg));
        Ok(())
    }

    fn receive_delegate(
        &self,
        target: &AgentId,
    ) -> Result<
        (
            DelegateMessage,
            Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
        ),
        QueueError,
    > {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey::Delegate { target: t, .. } => t == target,
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

    fn send_delegate_reply(
        &self,
        msg: CompleteMessage,
        reply_key: &RoutingKey,
    ) -> Result<(), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(reply_key.clone())
            .or_default()
            .push_back(ObservableMessage::Complete(msg));
        Ok(())
    }

    fn receive_delegate_reply(
        &self,
        reply_key: &RoutingKey,
    ) -> Result<
        (
            CompleteMessage,
            Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
        ),
        QueueError,
    > {
        let mut typed = self.typed_queues.lock().unwrap();

        if let Some(queue) = typed.get_mut(reply_key) {
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
    use crate::domain::{
        InferenceBackendType, Nonce, ObjectStorageType, Operation, RuntimeType, VectorStorageType,
    };
    use crate::domain::{Sequence, SessionId, SubmissionId, TimelineId};

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
            InvokeDiagnostics {
                harness_version: String::new(),
                history_turns: 0,
            },
        );
        let original_id = invoke.id.clone();

        queue.send_invoke(invoke).unwrap();

        // Receive typed message
        let (received, ack) = queue.receive_invoke(&test_agent_id()).unwrap();

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
            InvokeDiagnostics {
                harness_version: String::new(),
                history_turns: 0,
            },
        );

        queue.send_invoke(invoke).unwrap();

        let (received, _) = queue.receive_invoke(&test_agent_id()).unwrap();

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
            RequestDiagnostics {
                sequence: 0,
                endpoint: String::new(),
                request_bytes: 0,
                received_at_ms: 0,
            },
        );
        let original_id = request.id.clone();

        queue.send_request(request).unwrap();

        // Receive by service/backend/operation
        let (received, ack) = queue
            .receive_request(
                ServiceBackend::Kv(ObjectStorageType::Sqlite),
                Operation::Get,
            )
            .unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(
            received.service,
            ServiceBackend::Kv(ObjectStorageType::Sqlite)
        );
        assert_eq!(received.operation, Operation::Get);
        assert_eq!(received.payload.as_slice(), b"key");

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
            RequestDiagnostics {
                sequence: 0,
                endpoint: String::new(),
                request_bytes: 0,
                received_at_ms: 0,
            },
        );

        queue.send_request(request).unwrap();

        let (received, _) = queue
            .receive_request(
                ServiceBackend::Vec(VectorStorageType::SqliteVec),
                Operation::Search,
            )
            .unwrap();

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
            RequestDiagnostics {
                sequence: 0,
                endpoint: String::new(),
                request_bytes: 0,
                received_at_ms: 0,
            },
        );

        queue.send_request(request).unwrap();

        let (received, _) = queue
            .receive_request(
                ServiceBackend::Infer(InferenceBackendType::Ollama),
                Operation::Run,
            )
            .unwrap();

        assert_eq!(
            received.service,
            ServiceBackend::Infer(InferenceBackendType::Ollama)
        );
        assert_eq!(received.operation, Operation::Run);
    }

    // ========================================================================
    // Delegation tests (ADR 056)
    // ========================================================================

    #[test]
    fn receive_delegate_returns_typed_message() {
        let queue = InMemoryQueue::new();
        let nonce = Nonce::new("test-nonce");

        let delegate = DelegateMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"payload".to_vec(),
            nonce.clone(),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );
        let original_id = delegate.id.clone();

        queue.send_delegate(delegate).unwrap();

        let (received, ack) = queue.receive_delegate(&AgentId::new("summarizer")).unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.caller, AgentId::new("coordinator"));
        assert_eq!(received.target, AgentId::new("summarizer"));
        assert_eq!(received.payload, b"payload");
        assert_eq!(received.nonce, nonce);

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
            Nonce::generate(),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );

        queue.send_delegate(delegate).unwrap();

        let result = queue.receive_delegate(&AgentId::new("fact-checker"));
        assert!(matches!(result, Err(QueueError::Timeout)));
    }

    #[test]
    fn send_and_receive_delegate_reply() {
        let queue = InMemoryQueue::new();

        // Build a reply routing key (as if from a DelegateMessage)
        let reply_key = RoutingKey::DelegateReply {
            timeline: TimelineId::main(),
            submission: test_submission(),
            caller: AgentId::new("coordinator"),
            target: AgentId::new("summarizer"),
            nonce: Nonce::new("abc123"),
        };

        let complete = CompleteMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            HarnessType::Cli,
            b"result".to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        );

        queue.send_delegate_reply(complete, &reply_key).unwrap();

        let (received, ack) = queue.receive_delegate_reply(&reply_key).unwrap();
        assert_eq!(received.payload, b"result");
        ack().unwrap();
    }

    #[test]
    fn receive_delegate_reply_times_out_for_wrong_nonce() {
        let queue = InMemoryQueue::new();

        let reply_key_a = RoutingKey::DelegateReply {
            timeline: TimelineId::main(),
            submission: test_submission(),
            caller: AgentId::new("coordinator"),
            target: AgentId::new("summarizer"),
            nonce: Nonce::new("nonce-a"),
        };
        let reply_key_b = RoutingKey::DelegateReply {
            timeline: TimelineId::main(),
            submission: test_submission(),
            caller: AgentId::new("coordinator"),
            target: AgentId::new("summarizer"),
            nonce: Nonce::new("nonce-b"),
        };

        let complete = CompleteMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            HarnessType::Cli,
            b"result".to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        );

        queue.send_delegate_reply(complete, &reply_key_a).unwrap();

        let result = queue.receive_delegate_reply(&reply_key_b);
        assert!(matches!(result, Err(QueueError::Timeout)));
    }
}
