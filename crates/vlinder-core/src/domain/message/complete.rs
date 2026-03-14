//! CompleteMessage: Runtime → Harness (submission finished).

use serde::{Deserialize, Serialize};

use super::super::diagnostics::RuntimeDiagnostics;
use super::super::routing_key::{AgentId, RoutingKey};
use super::identity::{HarnessType, MessageId, SessionId, SubmissionId, TimelineId};
use super::PROTOCOL_VERSION;

/// Complete message: Runtime → Harness
///
/// Signals that a submission has finished.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompleteMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: TimelineId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_id: AgentId,
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

impl CompleteMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        harness: HarnessType,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: RuntimeDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline,
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
        RoutingKey::Complete {
            timeline: self.timeline.clone(),
            submission: self.submission.clone(),
            agent: self.agent_id.clone(),
            harness: self.harness,
        }
    }
}
