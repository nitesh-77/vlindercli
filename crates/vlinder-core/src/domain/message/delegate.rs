//! DelegateMessage: Agent → Agent (via runtime).

use serde::{Deserialize, Serialize};

use super::super::diagnostics::DelegateDiagnostics;
use super::super::routing_key::{AgentId, Nonce, RoutingKey};
use super::identity::{BranchId, MessageId, SessionId, SubmissionId};
use super::PROTOCOL_VERSION;

/// Delegate message: Agent → Agent (via runtime)
///
/// One agent invoking another. The platform routes through the queue,
/// dispatches the target, and sends the result to the reply subject.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelegateMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub caller: AgentId,
    pub target: AgentId,
    #[serde(skip)]
    pub payload: Vec<u8>,
    /// Uniqueness token for this delegation.
    pub nonce: Nonce,
    /// Caller's state hash at the time of delegation (ADR 055).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the runtime (ADR 071).
    #[serde(skip)]
    pub diagnostics: DelegateDiagnostics,
}

impl DelegateMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        caller: AgentId,
        target: AgentId,
        payload: Vec<u8>,
        nonce: Nonce,
        state: Option<String>,
        diagnostics: DelegateDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            branch,
            submission,
            session,
            caller,
            target,
            payload,
            nonce,
            state,
            diagnostics,
        }
    }

    /// Produce the routing key for this message (ADR 096 §4).
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey::Delegate {
            session: self.session.clone(),
            branch: self.branch,
            submission: self.submission.clone(),
            caller: self.caller.clone(),
            target: self.target.clone(),
        }
    }

    /// Produce the reply routing key for this delegation (ADR 096 §7).
    pub fn reply_routing_key(&self) -> RoutingKey {
        self.routing_key()
            .reply_key(Some(self.nonce.clone()))
            .unwrap()
    }
}
