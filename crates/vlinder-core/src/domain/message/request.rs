//! RequestMessage: Runtime → Service (agent calls a service).

use serde::Serialize;

use super::super::diagnostics::RequestDiagnostics;
use super::super::operation::Operation;
use super::super::routing_key::{AgentId, RoutingKey, ServiceBackend};
use super::identity::{MessageId, Sequence, SessionId, SubmissionId, TimelineId};
use super::response::ResponseMessage;
use super::{ExpectsReply, PROTOCOL_VERSION};

/// Request message: Runtime → Service
///
/// Agent requests a service operation (kv, vec, infer, embed).
/// Expects a ResponseMessage in response (enforced by ExpectsReply trait).
#[derive(Clone, Debug, Serialize)]
pub struct RequestMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: TimelineId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_id: AgentId,
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
}

impl RequestMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
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
            timeline,
            submission,
            session,
            agent_id,
            service,
            operation,
            sequence,
            payload,
            state,
            diagnostics,
        }
    }

    /// Produce the routing key for this message (ADR 096 §4).
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey::Request {
            timeline: self.timeline.clone(),
            submission: self.submission.clone(),
            agent: self.agent_id.clone(),
            service: self.service,
            operation: self.operation,
            sequence: self.sequence,
        }
    }
}

impl ExpectsReply for RequestMessage {
    type Reply = ResponseMessage;

    fn create_reply(&self, payload: Vec<u8>) -> ResponseMessage {
        ResponseMessage::from_request(self, payload)
    }
}
