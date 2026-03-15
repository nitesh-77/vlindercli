//! Session — conversation state for multi-turn interactions (ADR 054).
//!
//! A session groups multiple submissions into a conversation. The harness
//! wraps history into the payload so agents receive context automatically.

use serde::{Deserialize, Serialize};

use super::{SessionId, SubmissionId};

/// A conversation session tracking user/agent turns.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    /// Current unanswered question, or None when conversation is at rest.
    pub open: Option<String>,
    /// Session identifier (UUID).
    pub session: SessionId,
    /// Human-friendly name (petname, unique).
    pub name: String,
    /// Agent name this session is with.
    pub agent: String,
    /// Completed conversation turns.
    pub history: Vec<HistoryEntry>,
}

/// A single entry in the conversation history.
///
/// Uses untagged serde so User and Agent serialize as flat JSON objects
/// without a discriminator field.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HistoryEntry {
    User {
        user: String,
        submission: SubmissionId,
        at: String,
    },
    Agent {
        agent: String,
        at: String,
    },
}

impl Session {
    /// Create a new empty session with a generated petname.
    pub fn new(session: SessionId, agent: impl Into<String>) -> Self {
        let name = session.petname();
        let agent = agent.into();
        Self {
            open: None,
            session,
            name,
            agent,
            history: Vec::new(),
        }
    }

    /// Build the enriched payload that includes conversation history.
    ///
    /// Format: `User: ...\nAgent: ...\nUser: current_input`
    /// Agents receive this as a single string — they don't need to know
    /// about sessions, JSON, or git.
    pub fn build_payload(&self, current_input: &str) -> String {
        let mut parts = Vec::new();

        for entry in &self.history {
            match entry {
                HistoryEntry::User { user, .. } => {
                    parts.push(format!("User: {}", user));
                }
                HistoryEntry::Agent { agent, .. } => {
                    parts.push(format!("Agent: {}", agent));
                }
            }
        }

        parts.push(format!("User: {}", current_input));
        parts.join("\n")
    }

    /// Record that the user submitted input. Sets `open` to the question.
    pub fn record_user_input(&mut self, input: &str, submission: SubmissionId) {
        self.open = Some(input.to_string());
        self.history.push(HistoryEntry::User {
            user: input.to_string(),
            submission,
            at: String::new(),
        });
    }

    /// Record the agent's response. Clears `open` and appends the agent turn.
    pub fn record_agent_response(&mut self, response: &str) {
        self.open = None;
        self.history.push(HistoryEntry::Agent {
            agent: response.to_string(),
            at: String::new(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> Session {
        Session::new(
            SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap(),
            "pensieve",
        )
    }

    #[test]
    fn new_session_is_empty() {
        let session = test_session();

        assert!(session.open.is_none());
        assert_eq!(session.agent, "pensieve");
        assert!(session.history.is_empty());
        assert_eq!(
            session.session.as_str(),
            "d4761d76-dee4-4ebf-9df4-43b52efa4f78"
        );
    }

    #[test]
    fn build_payload_with_no_history() {
        let session = test_session();

        let payload = session.build_payload("hello");
        assert_eq!(payload, "User: hello");
    }

    #[test]
    fn build_payload_with_history() {
        let mut session = test_session();
        session.history.push(HistoryEntry::User {
            user: "summarize this article".to_string(),
            submission: SubmissionId::from("a1b2c3d".to_string()),
            at: "2026-02-08T14:30:05Z".to_string(),
        });
        session.history.push(HistoryEntry::Agent {
            agent: "This article discusses...".to_string(),
            at: "2026-02-08T14:30:47Z".to_string(),
        });

        let payload = session.build_payload("what about point 3?");
        assert_eq!(
            payload,
            "User: summarize this article\nAgent: This article discusses...\nUser: what about point 3?"
        );
    }

    #[test]
    fn record_user_input_sets_open() {
        let mut session = test_session();
        let sub = SubmissionId::from("a1b2c3d".to_string());

        session.record_user_input("hello", sub);

        assert_eq!(session.open, Some("hello".to_string()));
        assert_eq!(session.history.len(), 1);
        matches!(&session.history[0], HistoryEntry::User { user, .. } if user == "hello");
    }

    #[test]
    fn record_agent_response_clears_open() {
        let mut session = test_session();

        session.record_user_input("hello", SubmissionId::from("a1b2c3d".to_string()));
        assert!(session.open.is_some());

        session.record_agent_response("hi there");
        assert!(session.open.is_none());
        assert_eq!(session.history.len(), 2);
        matches!(&session.history[1], HistoryEntry::Agent { agent, .. } if agent == "hi there");
    }

    #[test]
    fn full_turn_cycle() {
        let mut session = test_session();

        // Turn 1
        session.record_user_input("summarize this", SubmissionId::from("sha1".to_string()));
        session.record_agent_response("Here's the summary...");

        // Turn 2
        session.record_user_input(
            "what about point 3?",
            SubmissionId::from("sha2".to_string()),
        );
        session.record_agent_response("Point 3 discusses...");

        assert!(session.open.is_none());
        assert_eq!(session.history.len(), 4);

        let payload = session.build_payload("thanks");
        assert!(payload.contains("User: summarize this"));
        assert!(payload.contains("Agent: Here's the summary..."));
        assert!(payload.contains("User: what about point 3?"));
        assert!(payload.contains("Agent: Point 3 discusses..."));
        assert!(payload.contains("User: thanks"));
    }

    #[test]
    fn json_round_trip() {
        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("a1b2c3d".to_string()));
        session.record_agent_response("hi there");

        let json = serde_json::to_string_pretty(&session).unwrap();
        let deserialized: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.session, session.session);
        assert_eq!(deserialized.agent, session.agent);
        assert!(deserialized.open.is_none());
        assert_eq!(deserialized.history.len(), 2);
    }

    #[test]
    fn json_schema_matches_adr() {
        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("a1b2c3d".to_string()));
        session.record_agent_response("hi there");

        let json = serde_json::to_string_pretty(&session).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Top-level fields
        assert!(value["open"].is_null());
        assert_eq!(value["session"], "d4761d76-dee4-4ebf-9df4-43b52efa4f78");
        assert_eq!(value["agent"], "pensieve");

        // History entries are untagged
        let history = value["history"].as_array().unwrap();
        assert!(history[0]["user"].is_string());
        assert!(history[0]["submission"].is_string());
        assert!(history[0]["at"].is_string());
        assert!(history[1]["agent"].is_string());
        assert!(history[1]["at"].is_string());
    }
}
