//! Message types for queue communication (ADR 044).
//!
//! Typed messages with explicit fields for observability:
//! - `InvokeMessageV2`: Harness → Runtime (start a submission, ADR 121)
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
pub mod observable_v2;
pub mod promote;
pub mod repair;
pub mod request;
pub mod response;
pub mod session_start;

// Re-export everything at the module level for backwards compatibility.
pub use complete::CompleteMessage;
pub use delegate::DelegateMessage;
pub use fork::ForkMessage;
pub use identity::{
    BranchId, DagNodeId, HarnessType, Instance, MessageId, Sequence, SequenceCounter, SessionId,
    StateHash, SubmissionId,
};
pub use invoke::InvokeMessageV2;
pub use observable::{MessageDetails, ObservableMessage, ObservableMessageHeaders};
pub use observable_v2::ObservableMessageV2;
pub use promote::PromoteMessage;
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
    use super::super::diagnostics::{DelegateDiagnostics, RequestDiagnostics, RuntimeDiagnostics};
    use super::super::operation::Operation;
    use super::super::routing_key::{
        AgentName, InferenceBackendType, Nonce, RoutingKey, RoutingKind, ServiceBackend,
    };
    use super::super::storage::ObjectStorageType;
    use super::*;

    fn test_submission() -> SubmissionId {
        SubmissionId::from("a1b2c3d".to_string())
    }

    fn test_agent_id() -> AgentName {
        AgentName::new("echo-agent")
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
    fn request_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = RequestMessage::new(
            BranchId::from(1),
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
            BranchId::from(1),
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
            BranchId::from(1),
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
    fn request_create_reply_returns_response() {
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
    fn observable_message_from_complete() {
        let complete = CompleteMessage::new(
            BranchId::from(1),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            HarnessType::Cli,
            b"test".to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        );
        let id = complete.id.clone();
        let submission = complete.submission.clone();

        let observable: ObservableMessage = complete.into();

        assert_eq!(observable.id(), &id);
        assert_eq!(observable.submission(), &submission);
        assert!(matches!(observable, ObservableMessage::Complete(_)));
    }

    #[test]
    fn observable_message_from_request() {
        let request = RequestMessage::new(
            BranchId::from(1),
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
        let complete = CompleteMessage::new(
            BranchId::from(1),
            submission.clone(),
            session.clone(),
            test_agent_id(),
            HarnessType::Cli,
            b"payload".to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        );

        let observable: ObservableMessage = complete.into();

        assert_eq!(observable.submission(), &submission);
        assert_eq!(observable.session(), &session);
        assert_eq!(observable.payload(), b"payload");
    }

    // --- Routing key tests (ADR 096 §4) ---

    #[test]
    fn request_routing_key() {
        let msg = RequestMessage::new(
            BranchId::from(1),
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
            RoutingKey {
                session: msg.session.clone(),
                branch: msg.branch,
                submission: msg.submission.clone(),
                kind: RoutingKind::Request {
                    agent: msg.agent_id.clone(),
                    service: msg.service,
                    operation: msg.operation,
                    sequence: msg.sequence,
                },
            }
        );
    }

    #[test]
    fn response_routing_key() {
        let request = RequestMessage::new(
            BranchId::from(1),
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
            RoutingKey {
                session: response.session.clone(),
                branch: response.branch,
                submission: response.submission.clone(),
                kind: RoutingKind::Response {
                    service: response.service,
                    agent: response.agent_id.clone(),
                    operation: response.operation,
                    sequence: response.sequence,
                },
            }
        );
    }

    #[test]
    fn complete_routing_key() {
        let msg = CompleteMessage::new(
            BranchId::from(1),
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
            RoutingKey {
                session: msg.session.clone(),
                branch: msg.branch,
                submission: msg.submission.clone(),
                kind: RoutingKind::Complete {
                    agent: msg.agent_id.clone(),
                    harness: msg.harness,
                },
            }
        );
    }

    #[test]
    fn delegate_routing_key() {
        let msg = DelegateMessage::new(
            BranchId::from(1),
            test_submission(),
            SessionId::new(),
            AgentName::new("coordinator"),
            AgentName::new("summarizer"),
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
            RoutingKey {
                session: msg.session.clone(),
                branch: msg.branch,
                submission: msg.submission.clone(),
                kind: RoutingKind::Delegate {
                    caller: msg.caller.clone(),
                    target: msg.target.clone(),
                },
            }
        );
    }

    #[test]
    fn delegate_reply_routing_key() {
        let msg = DelegateMessage::new(
            BranchId::from(1),
            test_submission(),
            SessionId::new(),
            AgentName::new("coordinator"),
            AgentName::new("summarizer"),
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
            RoutingKey {
                session: msg.session.clone(),
                branch: msg.branch,
                submission: msg.submission.clone(),
                kind: RoutingKind::DelegateReply {
                    caller: msg.caller.clone(),
                    target: msg.target.clone(),
                    nonce: Nonce::new("abc123"),
                },
            }
        );
    }

    #[test]
    fn delegate_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let nonce = Nonce::new("abc123");
        let msg = DelegateMessage::new(
            BranchId::from(1),
            submission.clone(),
            session.clone(),
            AgentName::new("coordinator"),
            AgentName::new("summarizer"),
            b"summarize this".to_vec(),
            nonce.clone(),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.caller, AgentName::new("coordinator"));
        assert_eq!(msg.target, AgentName::new("summarizer"));
        assert_eq!(msg.payload, b"summarize this");
        assert_eq!(msg.nonce, nonce);
    }

    #[test]
    fn observable_message_from_delegate() {
        let submission = test_submission();
        let delegate = DelegateMessage::new(
            BranchId::from(1),
            submission.clone(),
            SessionId::new(),
            AgentName::new("coordinator"),
            AgentName::new("summarizer"),
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
    fn request_reply_key_matches_response_routing_key() {
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
            test_request_diag(),
        );
        let response = ResponseMessage::from_request(&request, b"value".to_vec());

        let reply_key = request.routing_key().reply_key(None).unwrap();
        assert_eq!(reply_key, response.routing_key());
    }
}
