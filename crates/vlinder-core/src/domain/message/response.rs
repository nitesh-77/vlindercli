//! ResponseMessage: Service → Runtime (service replies).

use serde::Serialize;

use super::PROTOCOL_VERSION;
use super::identity::{MessageId, SubmissionId, SessionId, TimelineId, Sequence};
use super::request::RequestMessage;
use super::super::operation::Operation;
use super::super::routing_key::{AgentId, RoutingKey, ServiceBackend};
use super::super::service_payloads::ResponsePayload;
use super::super::diagnostics::ServiceDiagnostics;

/// Response message: Service → Runtime
///
/// Service responds to a request, echoing all dimensions for traceability.
#[derive(Debug, Serialize)]
pub struct ResponseMessage {
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
    pub payload: ResponsePayload,
    pub correlation_id: MessageId,
    /// State hash after this response (ADR 055).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the service worker (ADR 071).
    #[serde(skip)]
    pub diagnostics: ServiceDiagnostics,
}

impl ResponseMessage {
    /// Create a response from a request, echoing all dimensions.
    pub fn from_request(request: &RequestMessage, payload: Vec<u8>) -> Self {
        let placeholder = ServiceDiagnostics::storage(
            request.service.service_type(), request.service.backend_str(),
            request.operation, 0, 0,
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
            timeline: request.timeline.clone(),
            submission: request.submission.clone(),
            session: request.session.clone(),
            agent_id: request.agent_id.clone(),
            service: request.service,
            operation: request.operation,
            sequence: request.sequence,
            payload: ResponsePayload::Legacy(payload),
            correlation_id: request.id.clone(),
            state: None,
            diagnostics,
        }
    }

    /// Produce the routing key for this message (ADR 096 §4).
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey::Response {
            timeline: self.timeline.clone(),
            submission: self.submission.clone(),
            service: self.service,
            agent: self.agent_id.clone(),
            operation: self.operation,
            sequence: self.sequence,
        }
    }
}
