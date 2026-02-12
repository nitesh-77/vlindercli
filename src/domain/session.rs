//! Session — conversation state for multi-turn interactions (ADR 054).
//!
//! A session groups multiple submissions into a conversation. The harness
//! wraps history into the payload so agents receive context automatically.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::{SessionId, SubmissionId};

/// A conversation session tracking user/agent turns.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    /// Current unanswered question, or None when conversation is at rest.
    pub open: Option<String>,
    /// Session identifier grouping all turns.
    pub session: SessionId,
    /// Agent name this session is with.
    pub agent: String,
    /// Completed conversation turns.
    pub history: Vec<HistoryEntry>,
    /// Stable filename, computed once at creation time.
    #[serde(skip)]
    filename: String,
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
    /// Create a new empty session.
    pub fn new(session: SessionId, agent: impl Into<String>) -> Self {
        let agent = agent.into();
        let filename = make_filename(&session, &agent);
        Self {
            open: None,
            session,
            agent,
            history: Vec::new(),
            filename,
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
            at: format_utc_now(),
        });
    }

    /// Record the agent's response. Clears `open` and appends the agent turn.
    pub fn record_agent_response(&mut self, response: &str) {
        self.open = None;
        self.history.push(HistoryEntry::Agent {
            agent: response.to_string(),
            at: format_utc_now(),
        });
    }

    /// The stable filename for this session's JSON file.
    ///
    /// Format: `{datetime}_{agent}_{short_id}.json`
    /// Datetime uses filesystem-safe format: `2026-02-08T14-30-05Z`.
    /// Computed once at creation time — never changes.
    pub fn filename(&self) -> &str {
        &self.filename
    }
}

/// Format current time as UTC ISO 8601 string without external dependencies.
fn format_utc_now() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_timestamp(now.as_secs())
}

/// Format a unix timestamp as UTC ISO 8601 string.
fn format_unix_timestamp(secs: u64) -> String {
    // Manual UTC formatting to avoid chrono dependency
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Civil date from days since epoch (algorithm from Howard Hinnant)
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hours, minutes, seconds)
}

/// Compute the filename for a session. Called once at creation time.
fn make_filename(session: &SessionId, agent: &str) -> String {
    let datetime = format_utc_datetime_filesafe();
    let short_id = session.as_str()
        .strip_prefix("ses-")
        .unwrap_or(session.as_str());
    let short_id = if short_id.len() > 8 {
        &short_id[..8]
    } else {
        short_id
    };
    format!("{}_{}_{}.json", datetime, agent, short_id)
}

/// Format current UTC datetime for filenames.
///
/// Colons are invalid in filenames on Windows/macOS, so we replace them
/// with hyphens: `2026-02-08T14-30-05Z`.
fn format_utc_datetime_filesafe() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_timestamp(now.as_secs()).replace(':', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> Session {
        Session::new(
            SessionId::from("ses-abc12345".to_string()),
            "pensieve",
        )
    }

    #[test]
    fn new_session_is_empty() {
        let session = test_session();

        assert!(session.open.is_none());
        assert_eq!(session.agent, "pensieve");
        assert!(session.history.is_empty());
        assert_eq!(session.session.as_str(), "ses-abc12345");
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
        session.record_user_input("what about point 3?", SubmissionId::from("sha2".to_string()));
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
        assert_eq!(value["session"], "ses-abc12345");
        assert_eq!(value["agent"], "pensieve");

        // History entries are untagged
        let history = value["history"].as_array().unwrap();
        assert!(history[0]["user"].is_string());
        assert!(history[0]["submission"].is_string());
        assert!(history[0]["at"].is_string());
        assert!(history[1]["agent"].is_string());
        assert!(history[1]["at"].is_string());
    }

    #[test]
    fn filename_format() {
        let session = test_session();
        let filename = session.filename();

        // Format: {datetime}_{agent}_{short_id}.json
        // e.g. 2026-02-08T14-30-05Z_pensieve_abc12345.json
        assert!(filename.contains("_pensieve_"));
        assert!(filename.contains("abc12345"));
        assert!(filename.ends_with(".json"));
        // Datetime should include T and Z but no colons
        let datetime_part = filename.split('_').next().unwrap();
        assert!(datetime_part.contains('T'));
        assert!(datetime_part.ends_with('Z'));
        assert!(!datetime_part.contains(':'));
    }

    #[test]
    fn format_unix_timestamp_epoch() {
        assert_eq!(format_unix_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn format_unix_timestamp_known_date() {
        // 2026-02-08T23:30:05Z = 1770593405 seconds since epoch
        let ts = format_unix_timestamp(1770593405);
        assert_eq!(ts, "2026-02-08T23:30:05Z");
    }
}
