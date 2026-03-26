//! Message types for queue communication (ADR 044).
//!
//! Typed messages with explicit fields for observability:
//! - `InvokeMessage`: Harness → Runtime (start a submission, ADR 121)
//! - `RequestMessage`: Runtime → Service (agent calls a service)
//! - `ResponseMessage`: Service → Runtime (service replies)
//! - `CompleteMessage`: Runtime → Harness (submission finished)
//! - `DelegateMessage`: Agent → Agent (via runtime)
//! - `RepairMessage`: Platform → Sidecar (replay a failed service call, ADR 113)
//! - `ForkMessage`: CLI → Platform (create a timeline fork)
//! - `SessionStartMessage`: CLI → Platform (create a conversation session)

/// Serde helper: encode `Vec<u8>` as a base64 string for JSON-friendly transport.
pub(crate) mod base64_serde {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(d)?;
        STANDARD.decode(&encoded).map_err(serde::de::Error::custom)
    }
}

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
pub use complete::{CompleteMessage, DelegateReplyMessage};
pub use delegate::DelegateMessage;
pub use fork::ForkMessage;
pub use identity::{
    BranchId, DagNodeId, HarnessType, Instance, MessageId, Sequence, SequenceCounter, SessionId,
    StateHash, SubmissionId,
};
pub use invoke::InvokeMessage;
pub use observable::{MessageDetails, ObservableMessage, ObservableMessageHeaders};
pub use observable_v2::ObservableMessageV2;
pub use promote::PromoteMessage;
pub use repair::RepairMessage;
pub use request::RequestMessageV2;
pub use response::ResponseMessageV2;
pub use session_start::SessionStartMessage;

/// Protocol version stamped on every message at construction time.
pub const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::super::diagnostics::{DelegateDiagnostics, RuntimeDiagnostics};
    use super::super::routing_key::{AgentName, Nonce, RoutingKey, RoutingKind};
    use super::*;

    fn test_submission() -> SubmissionId {
        SubmissionId::from("a1b2c3d".to_string())
    }

    fn test_agent_id() -> AgentName {
        AgentName::new("echo-agent")
    }

    // --- Typed message tests ---

    #[test]
    fn complete_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = DelegateReplyMessage::new(
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

    // --- ObservableMessage tests ---

    #[test]
    fn observable_message_from_complete() {
        let complete = DelegateReplyMessage::new(
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
    fn observable_message_common_accessors() {
        let submission = test_submission();
        let session = SessionId::new();
        let complete = DelegateReplyMessage::new(
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
    fn complete_routing_key() {
        let msg = DelegateReplyMessage::new(
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
}
