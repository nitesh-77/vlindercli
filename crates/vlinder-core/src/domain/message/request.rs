//! `RequestMessage`: Runtime → Service (agent calls a service).

use serde::{Deserialize, Serialize};

use super::super::diagnostics::RequestDiagnostics;
use super::super::operation::Operation;
use super::super::routing_key::{AgentName, RoutingKey, RoutingKind, ServiceBackend};
use super::identity::{BranchId, DagNodeId, MessageId, Sequence, SessionId, SubmissionId};
use super::response::ResponseMessage;
use super::{ExpectsReply, PROTOCOL_VERSION};

/// Request message: Runtime → Service
///
/// Agent requests a service operation (kv, vec, infer, embed).
/// Expects a `ResponseMessage` in response (enforced by `ExpectsReply` trait).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_id: AgentName,
    pub service: ServiceBackend,
    pub operation: Operation,
    pub sequence: Sequence,
    #[serde(skip)]
    pub payload: Vec<u8>,
    /// State hash at the time this request was made (ADR 055).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the bridge (ADR 071).
    #[serde(skip)]
    pub diagnostics: RequestDiagnostics,
    /// SDK-supplied checkpoint name for durable execution (set after construction).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
}

impl RequestMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentName,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: RequestDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            branch,
            submission,
            session,
            agent_id,
            service,
            operation,
            sequence,
            payload,
            state,
            diagnostics,
            checkpoint: None,
        }
    }

    /// Produce the routing key for this message (ADR 096 §4).
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey {
            session: self.session.clone(),
            branch: self.branch,
            submission: self.submission.clone(),
            kind: RoutingKind::Request {
                agent: self.agent_id.clone(),
                service: self.service,
                operation: self.operation,
                sequence: self.sequence,
            },
        }
    }
}

impl ExpectsReply for RequestMessage {
    type Reply = ResponseMessage;

    fn create_reply(&self, payload: Vec<u8>) -> ResponseMessage {
        ResponseMessage::from_request(self, payload)
    }
}

/// Data-plane request payload — everything NOT in the subject (ADR 121).
///
/// The subject carries routing (session, branch, submission, agent, service,
/// operation, sequence) and protocol version. This struct carries the domain
/// data that goes in the NATS payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestMessageV2 {
    pub id: MessageId,
    pub dag_id: DagNodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub diagnostics: RequestDiagnostics,
    #[serde(with = "super::base64_serde")]
    pub payload: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_v2_json_round_trip() {
        let msg = RequestMessageV2 {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            state: Some("state-abc".to_string()),
            diagnostics: RequestDiagnostics {
                sequence: 1,
                endpoint: "/kv".to_string(),
                request_bytes: 42,
                received_at_ms: 1000,
            },
            payload: b"hello".to_vec(),
            checkpoint: Some("cp-1".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: RequestMessageV2 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn request_v2_payload_is_base64() {
        let msg = RequestMessageV2 {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            state: None,
            diagnostics: RequestDiagnostics {
                sequence: 1,
                endpoint: "/kv".to_string(),
                request_bytes: 0,
                received_at_ms: 0,
            },
            payload: b"hello".to_vec(),
            checkpoint: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(raw["payload"].as_str().unwrap(), "aGVsbG8=");
    }
}
