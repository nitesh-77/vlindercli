//! `InvokeMessage`: Harness → Runtime (start a submission).

use serde::{Deserialize, Serialize};

use super::super::diagnostics::{InvokeDiagnostics, RuntimeDiagnostics};
use super::super::routing_key::{AgentName, RoutingKey, RoutingKind};
use super::super::RuntimeType;
use super::complete::CompleteMessage;
use super::identity::{BranchId, DagNodeId, HarnessType, MessageId, SessionId, SubmissionId};
use super::{ExpectsReply, PROTOCOL_VERSION};

/// Serde helper: encode Vec<u8> as a base64 string for JSON-friendly transport.
mod base64_serde {
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

/// Invoke message: Harness → Runtime
///
/// Starts a submission by invoking an agent.
/// Expects a `CompleteMessage` in response (enforced by `ExpectsReply` trait).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvokeMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub harness: HarnessType,
    pub runtime: RuntimeType,
    pub agent_id: AgentName,
    #[serde(with = "base64_serde")]
    pub payload: Vec<u8>,
    /// Initial state hash from the previous turn's State trailer (ADR 055).
    /// None for the first invocation or when state tracking is not active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the harness (ADR 071).
    pub diagnostics: InvokeDiagnostics,
    /// `DagNode` to parent this invoke on in the DAG.
    /// The harness populates this from the conversations repo HEAD.
    pub dag_parent: DagNodeId,
}

impl InvokeMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent_id: AgentName,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: InvokeDiagnostics,
        dag_parent: DagNodeId,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            branch,
            submission,
            session,
            harness,
            runtime,
            agent_id,
            payload,
            state,
            diagnostics,
            dag_parent,
        }
    }

    /// Produce the routing key for this message (ADR 096 §4).
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey {
            session: self.session.clone(),
            branch: self.branch,
            submission: self.submission.clone(),
            kind: RoutingKind::Invoke {
                harness: self.harness,
                runtime: self.runtime,
                agent: self.agent_id.clone(),
            },
        }
    }

    /// Create a reply with the final state hash (ADR 055).
    pub fn create_reply_with_state(
        &self,
        payload: Vec<u8>,
        state: Option<String>,
    ) -> CompleteMessage {
        CompleteMessage::new(
            self.branch,
            self.submission.clone(),
            self.session.clone(),
            self.agent_id.clone(),
            self.harness,
            payload,
            state,
            RuntimeDiagnostics::placeholder(0),
        )
    }

    /// Create a reply with state and real runtime diagnostics (ADR 073).
    pub fn create_reply_with_diagnostics(
        &self,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: RuntimeDiagnostics,
    ) -> CompleteMessage {
        CompleteMessage::new(
            self.branch,
            self.submission.clone(),
            self.session.clone(),
            self.agent_id.clone(),
            self.harness,
            payload,
            state,
            diagnostics,
        )
    }
}

impl ExpectsReply for InvokeMessage {
    type Reply = CompleteMessage;

    fn create_reply(&self, payload: Vec<u8>) -> CompleteMessage {
        CompleteMessage::new(
            self.branch,
            self.submission.clone(),
            self.session.clone(),
            self.agent_id.clone(),
            self.harness,
            payload,
            None,
            RuntimeDiagnostics::placeholder(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_message_json_round_trip() {
        let msg = InvokeMessage::new(
            BranchId::from(1),
            SubmissionId::from("sub-1".to_string()),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentName::new("echo"),
            b"hello world".to_vec(),
            Some("abc123".to_string()),
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        );

        let json = serde_json::to_string(&msg).unwrap();
        let back: InvokeMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(back.id.as_str(), msg.id.as_str());
        assert_eq!(back.protocol_version, msg.protocol_version);
        assert_eq!(back.branch, msg.branch);
        assert_eq!(back.submission, msg.submission);
        assert_eq!(back.session, msg.session);
        assert_eq!(back.harness, msg.harness);
        assert_eq!(back.runtime, msg.runtime);
        assert_eq!(back.agent_id, msg.agent_id);
        assert_eq!(back.payload, b"hello world");
        assert_eq!(back.state, Some("abc123".to_string()));
        assert_eq!(back.diagnostics, msg.diagnostics);
    }

    #[test]
    fn payload_serializes_as_base64_string() {
        let msg = InvokeMessage::new(
            BranchId::from(1),
            SubmissionId::from("sub-1".to_string()),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentName::new("echo"),
            b"hello world".to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        );

        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();

        // payload should be a base64 string, not an array of integers
        let payload_val = &raw["payload"];
        assert!(
            payload_val.is_string(),
            "payload should be a string, got {payload_val:?}"
        );
        assert_eq!(payload_val.as_str().unwrap(), "aGVsbG8gd29ybGQ=");
    }
}
