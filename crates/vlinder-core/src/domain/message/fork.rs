//! ForkMessage: CLI → Platform (create a timeline fork).
//!
//! A control plane message that creates a new timeline branch in the DAG.
//! Both projections (SQL DagStore and git repo) react to this message:
//! - SQL: creates a Timeline row with parent_id and fork_point
//! - Git: creates a branch and updates timeline index files
//!
//! Unlike service messages, ForkMessage carries no payload — the fork point
//! hash and branch name are all that's needed to define the topology change.

use super::identity::{BranchId, DagNodeId, MessageId, SessionId, SubmissionId};
use super::PROTOCOL_VERSION;

/// Fork message: CLI → Platform
///
/// Creates a new timeline branch from a point in the DAG. The fork point
/// is a canonical DagNode hash in the source session. The new timeline
/// inherits the session's history up to the fork point.
///
/// This is a control plane message — there is no reply. The CLI confirms
/// success by querying the DagStore after the message is processed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ForkMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: BranchId,
    pub submission: SubmissionId,
    pub session: SessionId,
    /// Agent that owns the session being forked (needed for git tree path).
    pub agent_name: String,
    /// Branch name for the new timeline (e.g., "repair-infer-3").
    pub branch_name: String,
    /// The DagNode to fork from.
    pub fork_point: DagNodeId,
}

impl ForkMessage {
    pub fn new(
        timeline: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_name: String,
        branch_name: String,
        fork_point: DagNodeId,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline,
            submission,
            session,
            agent_name,
            branch_name,
            fork_point,
        }
    }
}
