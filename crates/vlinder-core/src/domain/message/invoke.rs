//! InvokeMessage: Harness → Runtime (start a submission).

use serde::Serialize;

use super::super::diagnostics::{InvokeDiagnostics, RuntimeDiagnostics};
use super::super::routing_key::{AgentId, RoutingKey};
use super::super::RuntimeType;
use super::complete::CompleteMessage;
use super::identity::{HarnessType, MessageId, SessionId, SubmissionId, TimelineId};
use super::{ExpectsReply, PROTOCOL_VERSION};

/// Invoke message: Harness → Runtime
///
/// Starts a submission by invoking an agent.
/// Expects a CompleteMessage in response (enforced by ExpectsReply trait).
#[derive(Clone, Debug, Serialize)]
pub struct InvokeMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: TimelineId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub harness: HarnessType,
    pub runtime: RuntimeType,
    pub agent_id: AgentId,
    #[serde(skip)]
    pub payload: Vec<u8>,
    /// Initial state hash from the previous turn's State trailer (ADR 055).
    /// None for the first invocation or when state tracking is not active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the harness (ADR 071).
    #[serde(skip)]
    pub diagnostics: InvokeDiagnostics,
}

impl InvokeMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent_id: AgentId,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: InvokeDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline,
            submission,
            session,
            harness,
            runtime,
            agent_id,
            payload,
            state,
            diagnostics,
        }
    }

    /// Produce the routing key for this message (ADR 096 §4).
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey::Invoke {
            timeline: self.timeline.clone(),
            submission: self.submission.clone(),
            harness: self.harness,
            runtime: self.runtime,
            agent: self.agent_id.clone(),
        }
    }

    /// Create a reply with the final state hash (ADR 055).
    pub fn create_reply_with_state(
        &self,
        payload: Vec<u8>,
        state: Option<String>,
    ) -> CompleteMessage {
        CompleteMessage::new(
            self.timeline.clone(),
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
            self.timeline.clone(),
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
            self.timeline.clone(),
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
