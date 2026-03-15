//! Session — conversation state for multi-turn interactions (ADR 054).
//!
//! A session groups multiple submissions into a conversation. History is
//! derived from the DAG — the session itself only tracks identity.

use serde::{Deserialize, Serialize};

use super::SessionId;

/// A conversation session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    /// Session identifier (UUID).
    pub session: SessionId,
    /// Human-friendly name (petname, unique).
    pub name: String,
    /// Agent name this session is with.
    pub agent: String,
}

impl Session {
    /// Create a new session with a generated petname.
    pub fn new(session: SessionId, agent: impl Into<String>) -> Self {
        let name = session.petname();
        let agent = agent.into();
        Self {
            session,
            name,
            agent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session() {
        let session = Session::new(
            SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap(),
            "pensieve",
        );

        assert_eq!(session.agent, "pensieve");
        assert_eq!(
            session.session.as_str(),
            "d4761d76-dee4-4ebf-9df4-43b52efa4f78"
        );
    }
}
