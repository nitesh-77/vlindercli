//! Message types for queue communication (ADR 044).
//!
//! Typed messages with explicit fields for observability:
//! - `InvokeMessage`: Harness → Runtime (start a submission)
//! - `RequestMessage`: Runtime → Service (agent calls a service)
//! - `ResponseMessage`: Service → Runtime (service replies)
//! - `CompleteMessage`: Runtime → Harness (submission finished)
//! - `DelegateMessage`: Agent → Agent (via runtime)
//! - `RepairMessage`: Platform → Sidecar (replay a failed service call, ADR 113)
//! - `ForkMessage`: CLI → Platform (create a timeline fork)
//! - `SessionStartMessage`: CLI → Platform (create a conversation session)

pub mod complete;
pub mod delegate;
pub mod fork;
pub mod identity;
pub mod invoke;
pub mod observable;
pub mod repair;
pub mod request;
pub mod response;
pub mod session_start;

// Re-export everything at the module level for backwards compatibility.
pub use complete::CompleteMessage;
pub use delegate::DelegateMessage;
pub use fork::ForkMessage;
pub use identity::{
    DagNodeId, HarnessType, MessageId, Sequence, SequenceCounter, SessionId, SubmissionId,
    TimelineId,
};
pub use invoke::InvokeMessage;
pub use observable::{ObservableMessage, ObservableMessageHeaders};
pub use repair::RepairMessage;
pub use request::RequestMessage;
pub use response::ResponseMessage;
pub use session_start::SessionStartMessage;

/// Protocol version stamped on every message at construction time.
pub const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Trait for messages that expect a reply.
///
/// The associated type `Reply` specifies what message type is expected
/// as a response. Terminal messages (Complete, Response) don't implement
/// this trait because they ARE replies.
pub trait ExpectsReply {
    type Reply;

    /// Create the reply for this message.
    fn create_reply(&self, payload: Vec<u8>) -> Self::Reply;
}

#[cfg(test)]
mod tests {
    use super::super::diagnostics::{
        DelegateDiagnostics, InvokeDiagnostics, RequestDiagnostics, RuntimeDiagnostics,
    };
    use super::super::operation::Operation;
    use super::super::routing_key::{
        AgentId, InferenceBackendType, Nonce, RoutingKey, ServiceBackend,
    };
    use super::super::storage::ObjectStorageType;
    use super::super::RuntimeType;
    use super::*;

    fn test_submission() -> SubmissionId {
        SubmissionId::from("a1b2c3d".to_string())
    }

    fn test_agent_id() -> AgentId {
        AgentId::new("echo-agent")
    }

    fn test_invoke_diag() -> InvokeDiagnostics {
        InvokeDiagnostics {
            harness_version: "0.1.0".to_string(),
            history_turns: 0,
        }
    }

    fn test_request_diag() -> RequestDiagnostics {
        RequestDiagnostics {
            sequence: 1,
            endpoint: "/test".to_string(),
            request_bytes: 0,
            received_at_ms: 0,
        }
    }

    // --- Typed message tests ---

    #[test]
    fn invoke_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = InvokeMessage::new(
            TimelineId::main(),
            submission.clone(),
            session.clone(),
            HarnessType::Cli,
            RuntimeType::Container,
            agent_id.clone(),
            b"hello".to_vec(),
            None,
            test_invoke_diag(),
            String::new(),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.harness, HarnessType::Cli);
        assert_eq!(msg.runtime, RuntimeType::Container);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.payload, b"hello");
        assert_eq!(msg.state, None);
    }

    #[test]
    fn request_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = RequestMessage::new(
            TimelineId::main(),
            submission.clone(),
            session.clone(),
            agent_id.clone(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::first(),
            b"key".to_vec(),
            None,
            test_request_diag(),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.service, ServiceBackend::Kv(ObjectStorageType::Sqlite));
        assert_eq!(msg.operation, Operation::Get);
        assert_eq!(msg.sequence.as_u32(), 1);
        assert_eq!(msg.payload.as_slice(), b"key");
    }

    #[test]
    fn response_from_request_echoes_dimensions() {
        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::from(3),
            b"key".to_vec(),
            None,
            test_request_diag(),
        );

        let response = ResponseMessage::from_request(&request, b"value".to_vec());

        assert_eq!(response.submission, request.submission);
        assert_eq!(response.session, request.session);
        assert_eq!(response.agent_id, request.agent_id);
        assert_eq!(response.service, request.service);
        assert_eq!(response.operation, request.operation);
        assert_eq!(response.sequence, request.sequence);
        assert_ne!(response.id, request.id);
        assert_eq!(response.correlation_id, request.id);
        assert_eq!(response.payload.as_slice(), b"value");
    }

    #[test]
    fn complete_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = CompleteMessage::new(
            TimelineId::main(),
            submission.clone(),
            session.clone(),
            agent_id.clone(),
            HarnessType::Cli,
            b"result".to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.harness, HarnessType::Cli);
        assert_eq!(msg.payload, b"result");
        assert_eq!(msg.state, None);
    }

    // --- ExpectsReply trait tests ---

    #[test]
    fn invoke_create_reply_returns_complete() {
        let invoke = InvokeMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"input".to_vec(),
            None,
            test_invoke_diag(),
            String::new(),
        );

        let complete: CompleteMessage = invoke.create_reply(b"output".to_vec());

        assert_eq!(complete.submission, invoke.submission);
        assert_eq!(complete.session, invoke.session);
        assert_eq!(complete.agent_id, invoke.agent_id);
        assert_eq!(complete.harness, invoke.harness);
        assert_eq!(complete.payload, b"output");
    }

    #[test]
    fn request_create_reply_returns_response() {
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
            test_request_diag(),
        );

        let response: ResponseMessage = request.create_reply(b"value".to_vec());

        assert_eq!(response.submission, request.submission);
        assert_eq!(response.session, request.session);
        assert_eq!(response.agent_id, request.agent_id);
        assert_eq!(response.service, request.service);
        assert_eq!(response.operation, request.operation);
        assert_eq!(response.sequence, request.sequence);
        assert_eq!(response.correlation_id, request.id);
        assert_eq!(response.payload.as_slice(), b"value");
    }

    // --- ObservableMessage tests ---

    #[test]
    fn observable_message_from_invoke() {
        let invoke = InvokeMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"test".to_vec(),
            None,
            test_invoke_diag(),
            String::new(),
        );
        let id = invoke.id.clone();
        let submission = invoke.submission.clone();

        let observable: ObservableMessage = invoke.into();

        assert_eq!(observable.id(), &id);
        assert_eq!(observable.submission(), &submission);
        assert!(matches!(observable, ObservableMessage::Invoke(_)));
    }

    #[test]
    fn observable_message_from_request() {
        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::first(),
            b"test".to_vec(),
            None,
            test_request_diag(),
        );
        let id = request.id.clone();

        let observable: ObservableMessage = request.into();

        assert_eq!(observable.id(), &id);
        assert!(matches!(observable, ObservableMessage::Request(_)));
    }

    #[test]
    fn observable_message_common_accessors() {
        let submission = test_submission();
        let session = SessionId::new();
        let invoke = InvokeMessage::new(
            TimelineId::main(),
            submission.clone(),
            session.clone(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"payload".to_vec(),
            None,
            test_invoke_diag(),
            String::new(),
        );

        let observable: ObservableMessage = invoke.into();

        assert_eq!(observable.submission(), &submission);
        assert_eq!(observable.session(), &session);
        assert_eq!(observable.payload(), b"payload");
    }

    // --- Routing key tests (ADR 096 §4) ---

    #[test]
    fn invoke_routing_key() {
        let msg = InvokeMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"test".to_vec(),
            None,
            test_invoke_diag(),
            String::new(),
        );

        let key = msg.routing_key();
        assert_eq!(
            key,
            RoutingKey::Invoke {
                timeline: msg.timeline.clone(),
                submission: msg.submission.clone(),
                harness: msg.harness,
                runtime: msg.runtime,
                agent: msg.agent_id.clone(),
            }
        );
    }

    #[test]
    fn request_routing_key() {
        let msg = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::first(),
            b"key".to_vec(),
            None,
            test_request_diag(),
        );

        let key = msg.routing_key();
        assert_eq!(
            key,
            RoutingKey::Request {
                timeline: msg.timeline.clone(),
                submission: msg.submission.clone(),
                agent: msg.agent_id.clone(),
                service: msg.service,
                operation: msg.operation,
                sequence: msg.sequence,
            }
        );
    }

    #[test]
    fn response_routing_key() {
        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
            Operation::Run,
            Sequence::from(3),
            b"prompt".to_vec(),
            None,
            test_request_diag(),
        );
        let response = ResponseMessage::from_request(&request, b"reply".to_vec());

        let key = response.routing_key();
        assert_eq!(
            key,
            RoutingKey::Response {
                timeline: response.timeline.clone(),
                submission: response.submission.clone(),
                service: response.service,
                agent: response.agent_id.clone(),
                operation: response.operation,
                sequence: response.sequence,
            }
        );
    }

    #[test]
    fn complete_routing_key() {
        let msg = CompleteMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            HarnessType::Web,
            b"done".to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        );

        let key = msg.routing_key();
        assert_eq!(
            key,
            RoutingKey::Complete {
                timeline: msg.timeline.clone(),
                submission: msg.submission.clone(),
                agent: msg.agent_id.clone(),
                harness: msg.harness,
            }
        );
    }

    #[test]
    fn delegate_routing_key() {
        let msg = DelegateMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"task".to_vec(),
            Nonce::new("test-nonce"),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );

        let key = msg.routing_key();
        assert_eq!(
            key,
            RoutingKey::Delegate {
                timeline: msg.timeline.clone(),
                submission: msg.submission.clone(),
                caller: msg.caller.clone(),
                target: msg.target.clone(),
            }
        );
    }

    #[test]
    fn delegate_reply_routing_key() {
        let msg = DelegateMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"task".to_vec(),
            Nonce::new("abc123"),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );

        let reply_key = msg.reply_routing_key();
        assert_eq!(
            reply_key,
            RoutingKey::DelegateReply {
                timeline: msg.timeline.clone(),
                submission: msg.submission.clone(),
                caller: msg.caller.clone(),
                target: msg.target.clone(),
                nonce: Nonce::new("abc123"),
            }
        );
    }

    #[test]
    fn delegate_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let nonce = Nonce::new("abc123");
        let msg = DelegateMessage::new(
            TimelineId::main(),
            submission.clone(),
            session.clone(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"summarize this".to_vec(),
            nonce.clone(),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.caller, AgentId::new("coordinator"));
        assert_eq!(msg.target, AgentId::new("summarizer"));
        assert_eq!(msg.payload, b"summarize this");
        assert_eq!(msg.nonce, nonce);
    }

    #[test]
    fn observable_message_from_delegate() {
        let submission = test_submission();
        let delegate = DelegateMessage::new(
            TimelineId::main(),
            submission.clone(),
            SessionId::new(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"test".to_vec(),
            Nonce::generate(),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );
        let id = delegate.id.clone();

        let observable: ObservableMessage = delegate.into();

        assert_eq!(observable.id(), &id);
        assert_eq!(observable.submission(), &submission);
        assert_eq!(observable.payload(), b"test");
        assert!(matches!(observable, ObservableMessage::Delegate(_)));
    }

    #[test]
    fn routing_key_reply_key_round_trip() {
        let invoke = InvokeMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"input".to_vec(),
            None,
            test_invoke_diag(),
            String::new(),
        );
        let complete = invoke.create_reply(b"output".to_vec());

        let reply_key = invoke.routing_key().reply_key(None).unwrap();
        assert_eq!(reply_key, complete.routing_key());
    }

    #[test]
    fn request_reply_key_matches_response_routing_key() {
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
            test_request_diag(),
        );
        let response = ResponseMessage::from_request(&request, b"value".to_vec());

        let reply_key = request.routing_key().reply_key(None).unwrap();
        assert_eq!(reply_key, response.routing_key());
    }

    // --- ObservableMessageHeaders assemble tests ---

    #[test]
    fn invoke_headers_assemble_produces_invoke_message() {
        let headers = ObservableMessageHeaders::Invoke {
            id: MessageId::from("msg-1".to_string()),
            protocol_version: "0.1.0".to_string(),
            timeline: TimelineId::main(),
            submission: test_submission(),
            session: SessionId::from("ses-1".to_string()),
            harness: HarnessType::Cli,
            runtime: RuntimeType::Container,
            agent_id: test_agent_id(),
            state: Some("state-abc".to_string()),
            diagnostics: test_invoke_diag(),
            dag_parent: String::new(),
        };

        let msg = headers.assemble(b"hello".to_vec());

        assert!(matches!(&msg, ObservableMessage::Invoke(_)));
        if let ObservableMessage::Invoke(m) = &msg {
            assert_eq!(m.id, MessageId::from("msg-1".to_string()));
            assert_eq!(m.harness, HarnessType::Cli);
            assert_eq!(m.runtime, RuntimeType::Container);
            assert_eq!(m.agent_id, test_agent_id());
            assert_eq!(m.payload, b"hello");
            assert_eq!(m.state, Some("state-abc".to_string()));
            assert_eq!(m.submission, test_submission());
        }
    }
}
