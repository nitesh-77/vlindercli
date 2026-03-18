//! `RepairMessage`: Platform → Sidecar (replay a failed service call, ADR 113).

use super::super::operation::Operation;
use super::super::routing_key::{AgentId, RoutingKey, ServiceBackend};
use super::identity::{
    BranchId, DagNodeId, HarnessType, MessageId, Sequence, SessionId, SubmissionId,
};
use super::PROTOCOL_VERSION;

/// Repair message: Platform → Sidecar
///
/// Instructs the sidecar to replay a service call and deliver the response
/// to the agent's checkpoint handler. The agent does not know it's a repair —
/// the checkpoint handler processes the response identically.
///
/// Unlike `InvokeMessage`, `dag_parent` and `checkpoint` are always required.
/// Unlike `RequestMessage`, there are no diagnostics (the agent didn't initiate
/// this call).
///
/// Reply type: `CompleteMessage` (same as `InvokeMessage`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RepairMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_id: AgentId,
    pub harness: HarnessType,
    /// The fork point in the DAG (required).
    pub dag_parent: DagNodeId,
    /// Checkpoint handler name on the agent (required).
    pub checkpoint: String,
    pub service: ServiceBackend,
    pub operation: Operation,
    pub sequence: Sequence,
    pub payload: Vec<u8>,
    /// State hash at the time of the original request (ADR 055).
    pub state: Option<String>,
}

impl RepairMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        harness: HarnessType,
        dag_parent: DagNodeId,
        checkpoint: String,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
        payload: Vec<u8>,
        state: Option<String>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            branch,
            submission,
            session,
            agent_id,
            harness,
            dag_parent,
            checkpoint,
            service,
            operation,
            sequence,
            payload,
            state,
        }
    }

    /// Produce the routing key for this message.
    pub fn routing_key(&self) -> RoutingKey {
        RoutingKey::Repair {
            session: self.session.clone(),
            branch: self.branch,
            submission: self.submission.clone(),
            harness: self.harness,
            agent: self.agent_id.clone(),
        }
    }
}
