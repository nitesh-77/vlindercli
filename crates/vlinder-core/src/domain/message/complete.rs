//! `CompleteMessage`: Runtime → Harness (submission finished).

use serde::{Deserialize, Serialize};

use super::super::diagnostics::RuntimeDiagnostics;
use super::super::routing_key::{AgentName, RoutingKey, RoutingKind};
use super::identity::{BranchId, DagNodeId, HarnessType, MessageId, SessionId, SubmissionId};
use super::PROTOCOL_VERSION;

/// Complete message: Runtime → Harness
///
/// Signals that a submission has finished.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelegateReplyMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_id: AgentName,
    pub harness: HarnessType,
    #[serde(skip)]
    pub payload: Vec<u8>,
    /// Final state hash after this invocation (ADR 055).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the runtime (ADR 071).
    #[serde(skip)]
    pub diagnostics: RuntimeDiagnostics,
}

impl DelegateReplyMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentName,
        harness: HarnessType,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: RuntimeDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            branch,
            submission,
            session,
            agent_id,
            harness,
            payload,
            state,
            diagnostics,
        }
    }

    /// Produce the routing key for this message (ADR 096 §4).
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey {
            session: self.session.clone(),
            branch: self.branch,
            submission: self.submission.clone(),
            kind: RoutingKind::Complete {
                agent: self.agent_id.clone(),
                harness: self.harness,
            },
        }
    }
}

/// Data-plane complete payload — everything NOT in the subject (ADR 121).
///
/// The subject carries routing (session, branch, submission, agent, harness)
/// and protocol version. This struct carries the domain data that goes in the
/// NATS payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompleteMessageV2 {
    pub id: MessageId,
    pub dag_id: DagNodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub diagnostics: RuntimeDiagnostics,
    #[serde(with = "super::base64_serde")]
    pub payload: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_v2_json_round_trip() {
        let msg = CompleteMessageV2 {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            state: Some("state-abc".to_string()),
            diagnostics: RuntimeDiagnostics::placeholder(42),
            payload: b"hello world".to_vec(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: CompleteMessageV2 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn complete_v2_payload_is_base64() {
        let msg = CompleteMessageV2 {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            state: None,
            diagnostics: RuntimeDiagnostics::placeholder(0),
            payload: b"hello world".to_vec(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(raw["payload"].is_string());
        assert_eq!(raw["payload"].as_str().unwrap(), "aGVsbG8gd29ybGQ=");
    }

    #[test]
    fn complete_v2_omits_none_state() {
        let msg = CompleteMessageV2 {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            state: None,
            diagnostics: RuntimeDiagnostics::placeholder(0),
            payload: b"test".to_vec(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(raw.get("state").is_none());
    }
}
