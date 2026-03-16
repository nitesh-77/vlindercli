//! SessionStartMessage: CLI → Platform (create a conversation session).
//!
//! A control plane message that creates a new session in the DagStore.
//! The RecordingQueue persists the session row when it processes this
//! message, satisfying the FK constraint before any dag_nodes are inserted.
//!
//! Like ForkMessage, this is fire-and-forget — there is no reply.

use super::identity::{BranchId, MessageId, SessionId};
use super::PROTOCOL_VERSION;

/// Session start message: CLI → Platform
///
/// Creates a new conversation session. The session must exist in the store
/// before any InvokeMessage references it (FK: dag_nodes.session_id → sessions.id).
#[derive(Clone, Debug)]
pub struct SessionStartMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: BranchId,
    pub session: SessionId,
    pub agent_name: String,
}

impl SessionStartMessage {
    pub fn new(timeline: BranchId, session: SessionId, agent_name: String) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline,
            session,
            agent_name,
        }
    }
}
