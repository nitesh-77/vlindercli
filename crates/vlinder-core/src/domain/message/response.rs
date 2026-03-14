//! ResponseMessage: Service → Runtime (service replies).

use serde::{Deserialize, Serialize};

use super::super::diagnostics::ServiceDiagnostics;
use super::super::operation::Operation;
use super::super::routing_key::{AgentId, RoutingKey, ServiceBackend};
use super::identity::{MessageId, Sequence, SessionId, SubmissionId, TimelineId};
use super::request::RequestMessage;
use super::PROTOCOL_VERSION;

/// Response message: Service → Runtime
///
/// Service responds to a request, echoing all dimensions for traceability.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
            timeline: request.timeline.clone(),
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
