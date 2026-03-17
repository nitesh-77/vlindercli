//! PromoteMessage: CLI → Platform (promote a branch to main).
//!
//! A control plane message that makes a branch the canonical "main"
//! branch for its session. Both projections (SQL DagStore and git repo) react:
//! - SQL: renames old main to `broken-{date}`, sets `broken_at`;
//!   renames promoted branch to "main"
//! - Git: updates refs accordingly

use super::identity::{BranchId, MessageId, SessionId, SubmissionId};
use super::PROTOCOL_VERSION;

/// Promote message: CLI → Platform
///
/// Makes the specified branch the new "main" for its session. The current
/// main branch is sealed (renamed to `broken-{date}`, `broken_at` set).
///
/// This is a control plane message — there is no reply. The CLI confirms
/// success by querying the DagStore after the message is processed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PromoteMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_name: String,
}

impl PromoteMessage {
    pub fn new(
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_name: String,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            branch,
            submission,
            session,
            agent_name,
        }
    }
}
