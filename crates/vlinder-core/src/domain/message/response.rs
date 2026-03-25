//! `ResponseMessage`: Service → Runtime (service replies).

use serde::{Deserialize, Serialize};

use super::super::diagnostics::ServiceDiagnostics;
use super::super::operation::Operation;
use super::super::routing_key::{AgentName, RoutingKey, RoutingKind, ServiceBackend};
use super::identity::{BranchId, DagNodeId, MessageId, Sequence, SessionId, SubmissionId};
use super::request::RequestMessage;
use super::PROTOCOL_VERSION;

/// Response message: Service → Runtime
///
/// Service responds to a request, echoing all dimensions for traceability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseMessage {
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
    pub correlation_id: MessageId,
    /// State hash after this response (ADR 055).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the service worker (ADR 071).
    #[serde(skip)]
    pub diagnostics: ServiceDiagnostics,
    /// HTTP status code for the response (used by provider server).
    /// Defaults to 200. Workers set this to signal errors (e.g. 500).
    pub status_code: u16,
    /// Checkpoint name echoed from the request (ADR 111).
    /// Present only for durable-mode calls. Enables repair: the platform
    /// can re-execute only requests that have a checkpoint (named re-entry point).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
}

impl ResponseMessage {
    /// Create a response from a request, echoing all dimensions.
    pub fn from_request(request: &RequestMessage, payload: Vec<u8>) -> Self {
        let placeholder = ServiceDiagnostics::storage(
            request.service.service_type(),
            request.service.backend_str(),
            request.operation,
            0,
            0,
        );
        Self::from_request_with_diagnostics(request, payload, placeholder)
    }

    /// Create a response with real diagnostics from the service worker.
    pub fn from_request_with_diagnostics(
        request: &RequestMessage,
        payload: Vec<u8>,
        diagnostics: ServiceDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            branch: request.branch,
            submission: request.submission.clone(),
            session: request.session.clone(),
            agent_id: request.agent_id.clone(),
            service: request.service,
            operation: request.operation,
            sequence: request.sequence,
            payload,
            correlation_id: request.id.clone(),
            state: None,
            diagnostics,
            status_code: 200,
            checkpoint: request.checkpoint.clone(),
        }
    }

    /// Produce the routing key for this message (ADR 096 §4).
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey {
            session: self.session.clone(),
            branch: self.branch,
            submission: self.submission.clone(),
            kind: RoutingKind::Response {
                service: self.service,
                agent: self.agent_id.clone(),
                operation: self.operation,
                sequence: self.sequence,
            },
        }
    }
}

/// Data-plane response payload — everything NOT in the subject (ADR 121).
///
/// The subject carries routing (session, branch, submission, agent, service,
/// operation, sequence) and protocol version. This struct carries the domain
/// data that goes in the NATS payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseMessageV2 {
    pub id: MessageId,
    pub dag_id: DagNodeId,
    pub correlation_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub diagnostics: ServiceDiagnostics,
    #[serde(with = "super::base64_serde")]
    pub payload: Vec<u8>,
    pub status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_v2_json_round_trip() {
        let msg = ResponseMessageV2 {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            correlation_id: MessageId::from("req-1".to_string()),
            state: Some("state-abc".to_string()),
            diagnostics: ServiceDiagnostics::storage(
                super::super::super::ServiceType::Kv,
                "sqlite",
                Operation::Get,
                42,
                10,
            ),
            payload: b"hello".to_vec(),
            status_code: 200,
            checkpoint: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ResponseMessageV2 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn response_v2_payload_is_base64() {
        let msg = ResponseMessageV2 {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            correlation_id: MessageId::from("req-1".to_string()),
            state: None,
            diagnostics: ServiceDiagnostics::storage(
                super::super::super::ServiceType::Kv,
                "sqlite",
                Operation::Get,
                0,
                0,
            ),
            payload: b"hello".to_vec(),
            status_code: 200,
            checkpoint: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(raw["payload"].as_str().unwrap(), "aGVsbG8=");
    }
}
