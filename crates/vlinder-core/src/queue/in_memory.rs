//! In-memory queue implementation.

use crate::domain::{
    Acknowledgement, AgentName, CompleteMessage, DataMessageKind, DataRoutingKey, DelegateMessage,
    DelegateReplyMessage, ForkMessage, HarnessType, InvokeMessage, MessageQueue, ObservableMessage,
    ObservableMessageV2, Operation, QueueError, RepairMessage, RequestMessage, RequestMessageV2,
    ResponseMessage, ResponseMessageV2, RoutingKey, RoutingKind, Sequence, ServiceBackend,
    SubmissionId,
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
    /// Data-plane messages keyed by `DataRoutingKey` (ADR 121).
    data_queues: Arc<Mutex<HashMap<DataRoutingKey, VecDeque<ObservableMessageV2>>>>,
    /// Legacy messages keyed by `RoutingKey` (ADR 096 §5).
    pub(crate) typed_queues: Arc<Mutex<HashMap<RoutingKey, VecDeque<ObservableMessage>>>>,
}

impl InMemoryQueue {
    pub fn new() -> Self {
        Self {
            data_queues: Arc::new(Mutex::new(HashMap::new())),
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
    fn send_invoke(&self, key: DataRoutingKey, msg: InvokeMessage) -> Result<(), QueueError> {
        let v2 = ObservableMessageV2::InvokeV2 {
            key: key.clone(),
            msg,
        };
        let mut data = self
            .data_queues
            .lock()
            .map_err(|e| QueueError::SendFailed(format!("lock poisoned: {e}")))?;
        data.entry(key).or_default().push_back(v2);
        Ok(())
    }

    fn receive_invoke(
        &self,
        agent: &AgentName,
    ) -> Result<(DataRoutingKey, InvokeMessage, Acknowledgement), QueueError> {
        let mut data = self
            .data_queues
            .lock()
            .map_err(|e| QueueError::ReceiveFailed(format!("lock poisoned: {e}")))?;

        for (key, queue) in data.iter_mut() {
            let DataMessageKind::Invoke { agent: a, .. } = &key.kind else {
                continue;
            };
            if a == agent {
                if let Some(ObservableMessageV2::InvokeV2 { msg, .. }) = queue.pop_front() {
                    let key = key.clone();
                    return Ok((key, msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn send_complete(&self, key: DataRoutingKey, msg: CompleteMessage) -> Result<(), QueueError> {
        let v2 = ObservableMessageV2::CompleteV2 {
            key: key.clone(),
            msg,
        };
        let mut data = self
            .data_queues
            .lock()
            .map_err(|e| QueueError::SendFailed(format!("lock poisoned: {e}")))?;
        data.entry(key).or_default().push_back(v2);
        Ok(())
    }

    fn receive_complete(
        &self,
        submission: &SubmissionId,
        _harness: HarnessType,
    ) -> Result<(DataRoutingKey, CompleteMessage, Acknowledgement), QueueError> {
        let mut data = self
            .data_queues
            .lock()
            .map_err(|e| QueueError::ReceiveFailed(format!("lock poisoned: {e}")))?;

        for (key, queue) in data.iter_mut() {
            if key.submission != *submission {
                continue;
            }
            let DataMessageKind::Complete { .. } = &key.kind else {
                continue;
            };
            if let Some(ObservableMessageV2::CompleteV2 { msg, .. }) = queue.pop_front() {
                let key = key.clone();
                return Ok((key, msg, Box::new(|| Ok(()))));
            }
        }

        Err(QueueError::Timeout)
    }

    fn send_request_v2(
        &self,
        key: DataRoutingKey,
        msg: RequestMessageV2,
    ) -> Result<(), QueueError> {
        let v2 = ObservableMessageV2::RequestV2 {
            key: key.clone(),
            msg,
        };
        let mut data = self
            .data_queues
            .lock()
            .map_err(|e| QueueError::SendFailed(format!("lock poisoned: {e}")))?;
        data.entry(key).or_default().push_back(v2);
        Ok(())
    }

    fn receive_request_v2(
        &self,
        service: ServiceBackend,
        operation: Operation,
    ) -> Result<(DataRoutingKey, RequestMessageV2, Acknowledgement), QueueError> {
        let mut data = self
            .data_queues
            .lock()
            .map_err(|e| QueueError::ReceiveFailed(format!("lock poisoned: {e}")))?;

        for (key, queue) in data.iter_mut() {
            let DataMessageKind::Request {
                service: s,
                operation: o,
                ..
            } = &key.kind
            else {
                continue;
            };
            if *s == service && *o == operation {
                if let Some(ObservableMessageV2::RequestV2 { msg, .. }) = queue.pop_front() {
                    let key = key.clone();
                    return Ok((key, msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
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

    fn send_response_v2(
        &self,
        key: DataRoutingKey,
        msg: ResponseMessageV2,
    ) -> Result<(), QueueError> {
        let v2 = ObservableMessageV2::ResponseV2 {
            key: key.clone(),
            msg,
        };
        let mut data = self
            .data_queues
            .lock()
            .map_err(|e| QueueError::SendFailed(format!("lock poisoned: {e}")))?;
        data.entry(key).or_default().push_back(v2);
        Ok(())
    }

    fn receive_response_v2(
        &self,
        submission: &SubmissionId,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
    ) -> Result<(DataRoutingKey, ResponseMessageV2, Acknowledgement), QueueError> {
        let mut data = self
            .data_queues
            .lock()
            .map_err(|e| QueueError::ReceiveFailed(format!("lock poisoned: {e}")))?;

        for (key, queue) in data.iter_mut() {
            if key.submission != *submission {
                continue;
            }
            let DataMessageKind::Response {
                service: s,
                operation: o,
                sequence: sq,
                ..
            } = &key.kind
            else {
                continue;
            };
            if *s == service && *o == operation && *sq == sequence {
                if let Some(ObservableMessageV2::ResponseV2 { msg, .. }) = queue.pop_front() {
                    let key = key.clone();
                    return Ok((key, msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
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

    fn receive_request(
        &self,
        service: ServiceBackend,
        operation: Operation,
    ) -> Result<(RequestMessage, Acknowledgement), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey {
                    kind:
                        RoutingKind::Request {
                            service: svc,
                            operation: op,
                            ..
                        },
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
    ) -> Result<(ResponseMessage, Acknowledgement), QueueError> {
        // Exact lookup via reply_key (ADR 096 §6).
        let reply_key = request
            .routing_key()
            .reply_key(None)
            .expect("Request routing key always produces a Response reply key");
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
        target: &AgentName,
    ) -> Result<(DelegateMessage, Acknowledgement), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey {
                    kind: RoutingKind::Delegate { target: ref t, .. },
                    ..
                } => t == target,
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
        msg: DelegateReplyMessage,
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
    ) -> Result<(DelegateReplyMessage, Acknowledgement), QueueError> {
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

    fn send_repair(&self, msg: RepairMessage) -> Result<(), QueueError> {
        let key = msg.routing_key();
        let mut typed = self.typed_queues.lock().unwrap();
        typed
            .entry(key)
            .or_default()
            .push_back(ObservableMessage::Repair(msg));
        Ok(())
    }

    fn receive_repair(
        &self,
        agent: &AgentName,
    ) -> Result<(RepairMessage, Acknowledgement), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        for (key, queue) in typed.iter_mut() {
            let matches = match key {
                RoutingKey {
                    kind: RoutingKind::Repair { agent: ref a, .. },
                    ..
                } => a == agent,
                _ => false,
            };
            if matches {
                if let Some(ObservableMessage::Repair(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();
                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn send_fork(&self, _msg: ForkMessage) -> Result<(), QueueError> {
        // Fork is fire-and-forget — no consumer subscribes.
        // RecordingQueue intercepts and records the DagNode before this is called.
        Ok(())
    }

    fn send_promote(&self, _msg: crate::domain::PromoteMessage) -> Result<(), QueueError> {
        // Promote is fire-and-forget — no consumer subscribes.
        // RecordingQueue intercepts and records the DagNode before this is called.
        Ok(())
    }

    fn send_session_start(
        &self,
        _msg: crate::domain::SessionStartMessage,
    ) -> Result<crate::domain::BranchId, QueueError> {
        // InMemoryQueue doesn't have a store — return a placeholder.
        // RecordingQueue wraps this and returns the real branch ID.
        Ok(crate::domain::BranchId::from(1))
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{BranchId, DagNodeId, MessageId, Sequence, SessionId, SubmissionId};
    use crate::domain::{
        InferenceBackendType, Nonce, ObjectStorageType, Operation, RoutingKind, RuntimeType,
        VectorStorageType,
    };

    fn test_agent_id() -> AgentName {
        AgentName::new("echo-agent")
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

        let key = DataRoutingKey {
            session: SessionId::new(),
            branch: BranchId::from(1),
            submission: test_submission(),
            kind: DataMessageKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: test_agent_id(),
            },
        };
        let msg = InvokeMessage {
            id: MessageId::from("msg-invoke-1".to_string()),
            dag_id: DagNodeId::root(),
            state: None,
            diagnostics: InvokeDiagnostics {
                harness_version: String::new(),
            },
            dag_parent: DagNodeId::root(),
            payload: b"hello".to_vec(),
        };
        let original_id = msg.id.clone();

        queue.send_invoke(key, msg).unwrap();

        // Receive typed message
        let (recv_key, received, ack) = queue.receive_invoke(&test_agent_id()).unwrap();

        assert_eq!(received.id, original_id);
        assert!(matches!(
            recv_key.kind,
            DataMessageKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                ..
            }
        ));
        assert_eq!(received.payload, b"hello");

        ack().unwrap();
    }

    #[test]
    fn receive_invoke_preserves_all_dimensions() {
        let queue = InMemoryQueue::new();

        let submission = test_submission();
        let agent_id = test_agent_id();

        let key = DataRoutingKey {
            session: SessionId::new(),
            branch: BranchId::from(1),
            submission: submission.clone(),
            kind: DataMessageKind::Invoke {
                harness: HarnessType::Web,
                runtime: RuntimeType::Container,
                agent: agent_id.clone(),
            },
        };
        let msg = InvokeMessage {
            id: MessageId::from("msg-invoke-2".to_string()),
            dag_id: DagNodeId::root(),
            state: None,
            diagnostics: InvokeDiagnostics {
                harness_version: String::new(),
            },
            dag_parent: DagNodeId::root(),
            payload: b"input".to_vec(),
        };

        queue.send_invoke(key, msg).unwrap();

        let (recv_key, _, _) = queue.receive_invoke(&test_agent_id()).unwrap();

        // All dimensions preserved for reply construction
        assert_eq!(recv_key.submission, submission);
        assert!(matches!(
            recv_key.kind,
            DataMessageKind::Invoke { ref agent, harness: HarnessType::Web, .. } if *agent == agent_id
        ));
    }

    #[test]
    fn receive_request_returns_typed_message() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            BranchId::from(1),
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
            BranchId::from(1),
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
            BranchId::from(1),
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
            BranchId::from(1),
            test_submission(),
            SessionId::new(),
            AgentName::new("coordinator"),
            AgentName::new("summarizer"),
            b"payload".to_vec(),
            nonce.clone(),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );
        let original_id = delegate.id.clone();

        queue.send_delegate(delegate).unwrap();

        let (received, ack) = queue
            .receive_delegate(&AgentName::new("summarizer"))
            .unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.caller, AgentName::new("coordinator"));
        assert_eq!(received.target, AgentName::new("summarizer"));
        assert_eq!(received.payload, b"payload");
        assert_eq!(received.nonce, nonce);

        ack().unwrap();
    }

    #[test]
    fn receive_delegate_times_out_for_wrong_target() {
        let queue = InMemoryQueue::new();

        let delegate = DelegateMessage::new(
            BranchId::from(1),
            test_submission(),
            SessionId::new(),
            AgentName::new("coordinator"),
            AgentName::new("summarizer"),
            b"payload".to_vec(),
            Nonce::generate(),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );

        queue.send_delegate(delegate).unwrap();

        let result = queue.receive_delegate(&AgentName::new("fact-checker"));
        assert!(matches!(result, Err(QueueError::Timeout)));
    }

    #[test]
    fn send_and_receive_delegate_reply() {
        let queue = InMemoryQueue::new();

        // Build a reply routing key (as if from a DelegateMessage)
        let reply_key = RoutingKey {
            session: SessionId::new(),
            branch: BranchId::from(1),
            submission: test_submission(),
            kind: RoutingKind::DelegateReply {
                caller: AgentName::new("coordinator"),
                target: AgentName::new("summarizer"),
                nonce: Nonce::new("abc123"),
            },
        };

        let complete = DelegateReplyMessage::new(
            BranchId::from(1),
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

        let reply_key_a = RoutingKey {
            session: SessionId::new(),
            branch: BranchId::from(1),
            submission: test_submission(),
            kind: RoutingKind::DelegateReply {
                caller: AgentName::new("coordinator"),
                target: AgentName::new("summarizer"),
                nonce: Nonce::new("nonce-a"),
            },
        };
        let reply_key_b = RoutingKey {
            session: SessionId::new(),
            branch: BranchId::from(1),
            submission: test_submission(),
            kind: RoutingKind::DelegateReply {
                caller: AgentName::new("coordinator"),
                target: AgentName::new("summarizer"),
                nonce: Nonce::new("nonce-b"),
            },
        };

        let complete = DelegateReplyMessage::new(
            BranchId::from(1),
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

    // ========================================================================
    // Invoke tests (ADR 121 — data plane)
    // ========================================================================

    fn test_data_routing_key() -> DataRoutingKey {
        DataRoutingKey {
            session: SessionId::new(),
            branch: BranchId::from(1),
            submission: test_submission(),
            kind: DataMessageKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: test_agent_id(),
            },
        }
    }

    fn test_invoke() -> InvokeMessage {
        InvokeMessage {
            id: crate::domain::MessageId::from("msg-1".to_string()),
            dag_id: crate::domain::DagNodeId::root(),
            state: Some("state-abc".to_string()),
            diagnostics: InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            dag_parent: crate::domain::DagNodeId::root(),
            payload: b"hello".to_vec(),
        }
    }

    #[test]
    fn send_and_receive_invoke() {
        let queue = InMemoryQueue::new();
        let key = test_data_routing_key();
        let msg = test_invoke();

        queue.send_invoke(key, msg.clone()).unwrap();

        let (recv_key, recv_msg, ack) = queue.receive_invoke(&test_agent_id()).unwrap();

        assert_eq!(recv_msg, msg);
        assert_eq!(recv_key.submission, test_submission());
        ack().unwrap();
    }

    #[test]
    fn receive_invoke_times_out_for_wrong_agent() {
        let queue = InMemoryQueue::new();
        let key = test_data_routing_key();
        let msg = test_invoke();

        queue.send_invoke(key, msg).unwrap();

        let result = queue.receive_invoke(&AgentName::new("other-agent"));
        assert!(matches!(result, Err(QueueError::Timeout)));
    }

    #[test]
    fn multiple_invoke_messages_delivered_in_order() {
        let queue = InMemoryQueue::new();

        let key = test_data_routing_key();
        let msg1 = InvokeMessage {
            id: crate::domain::MessageId::from("msg-first".to_string()),
            dag_id: crate::domain::DagNodeId::root(),
            state: None,
            diagnostics: InvokeDiagnostics {
                harness_version: String::new(),
            },
            dag_parent: crate::domain::DagNodeId::root(),
            payload: b"first".to_vec(),
        };
        let msg2 = InvokeMessage {
            id: crate::domain::MessageId::from("msg-second".to_string()),
            dag_id: crate::domain::DagNodeId::root(),
            state: None,
            diagnostics: InvokeDiagnostics {
                harness_version: String::new(),
            },
            dag_parent: crate::domain::DagNodeId::root(),
            payload: b"second".to_vec(),
        };

        queue.send_invoke(key.clone(), msg1.clone()).unwrap();
        queue.send_invoke(key, msg2.clone()).unwrap();

        let (_, recv1, _) = queue.receive_invoke(&test_agent_id()).unwrap();
        assert_eq!(recv1, msg1);

        let (_, recv2, _) = queue.receive_invoke(&test_agent_id()).unwrap();
        assert_eq!(recv2, msg2);
    }
}
