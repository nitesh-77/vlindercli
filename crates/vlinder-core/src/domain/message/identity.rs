//! Identity types carried by messages.
//!
//! Small newtypes for message dimensions: who, what, when, where.

use std::fmt;
use uuid::Uuid;

use serde::{Deserialize, Serialize};

// --- MessageId ---

/// Unique identifier for a message.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct MessageId(String);

impl MessageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for MessageId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// --- SubmissionId (ADR 044, ADR 054) ---

/// Unique identifier for a user-initiated submission.
///
/// Value is a git commit SHA — directly pasteable into `git show`.
/// A submission groups all messages related to a single user request.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubmissionId(String);

impl SubmissionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Create a placeholder SubmissionId for non-session contexts.
    ///
    /// In session mode, SubmissionIds are content-addressed hashes derived
    /// from the user input and session context. This factory exists for
    /// backwards compatibility and will be removed once all paths use
    /// content-addressed IDs.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Create a content-addressed SubmissionId (ADR 081).
    ///
    /// SHA-256 hash of (payload, session_id, parent_submission), producing
    /// a Merkle chain of user inputs. Same inputs at the same conversation
    /// position always produce the same SubmissionId — cherry-picks across
    /// branches preserve identity because the content hasn't changed.
    ///
    /// # Arguments
    /// * `payload` — the user's input (enriched with history)
    /// * `session_id` — groups the submission into a conversation
    /// * `parent_submission` — previous SubmissionId in this session (empty for first turn)
    pub fn content_addressed(payload: &[u8], session_id: &str, parent_submission: &str) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(payload);
        hasher.update(b"\0");
        hasher.update(session_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(parent_submission.as_bytes());
        let hash: Vec<u8> = hasher.finalize().to_vec();
        let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
        Self(hex)
    }
}

impl Default for SubmissionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SubmissionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SubmissionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// --- SessionId (ADR 054) ---

/// Unique identifier for a conversation session.
///
/// Format: `ses-{uuid}`. Groups multiple submissions into a conversation.
/// Created by the CLI when the REPL starts.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    /// Create a new session ID with "ses-" prefix.
    pub fn new() -> Self {
        Self(format!("ses-{}", Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// --- TimelineId (ADR 093) ---

/// Immutable identifier for a timeline (branch-scoped subjects).
///
/// Value is the integer primary key from the `timelines` table in the DAG store.
/// Carried as a string in NATS headers and message subjects.
///
/// `TimelineId::main()` returns `"1"` — row 1 is always the main timeline.
/// Timeline IDs never change, even when branch names are renamed during promote.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimelineId(String);

impl TimelineId {
    /// The main timeline (row 1 in the timelines table).
    pub fn main() -> Self {
        Self("1".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TimelineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for TimelineId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<i64> for TimelineId {
    fn from(id: i64) -> Self {
        Self(id.to_string())
    }
}

// --- Sequence (ADR 044) ---

/// Sequence number for ordering interactions within a submission.
///
/// Starts at 1 and increments for each service request.
/// Used to reconstruct the order of events when debugging.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Sequence(u32);

impl Sequence {
    /// Create the first sequence number (1).
    pub fn first() -> Self {
        Self(1)
    }

    /// Get the next sequence number.
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }

    /// Get the raw sequence number.
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

/// Thread-safe counter that allocates sequential Sequence values.
///
/// Each call to `next()` returns the current value and advances.
/// Starts at 1 (the first valid sequence number).
pub struct SequenceCounter(std::sync::Mutex<Sequence>);

impl SequenceCounter {
    /// Create a counter starting at sequence 1.
    pub fn new() -> Self {
        Self(std::sync::Mutex::new(Sequence::first()))
    }

    /// Allocate the next sequence number.
    pub fn next(&self) -> Sequence {
        let mut current = self.0.lock().unwrap();
        let seq = *current;
        *current = current.next();
        seq
    }

    /// Reset the counter back to sequence 1.
    pub fn reset(&self) {
        *self.0.lock().unwrap() = Sequence::first();
    }
}

impl Default for SequenceCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Sequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for Sequence {
    fn from(n: u32) -> Self {
        Self(n)
    }
}

// --- HarnessType (ADR 044) ---

/// The type of entry point that initiated a submission.
///
/// Each variant represents a concrete harness implementation.
/// Used to route completion messages back to the correct harness type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessType {
    /// Command-line interface harness
    Cli,
    /// Web API harness
    Web,
    /// REST API harness
    Api,
    /// WhatsApp integration harness
    Whatsapp,
    /// gRPC remote harness
    Grpc,
}

impl HarnessType {
    /// Get the string representation for use in subjects.
    pub fn as_str(&self) -> &'static str {
        match self {
            HarnessType::Cli => "cli",
            HarnessType::Web => "web",
            HarnessType::Api => "api",
            HarnessType::Whatsapp => "whatsapp",
            HarnessType::Grpc => "grpc",
        }
    }
}

impl std::str::FromStr for HarnessType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cli" => Ok(HarnessType::Cli),
            "web" => Ok(HarnessType::Web),
            "api" => Ok(HarnessType::Api),
            "whatsapp" => Ok(HarnessType::Whatsapp),
            "grpc" => Ok(HarnessType::Grpc),
            _ => Err(format!("unknown harness type: {}", s)),
        }
    }
}

impl fmt::Display for HarnessType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::str::FromStr;

    // --- SubmissionId tests ---

    #[test]
    fn submission_id_from_string() {
        let id = SubmissionId::from("a1b2c3d".to_string());
        assert_eq!(id.as_str(), "a1b2c3d");
    }

    #[test]
    fn submission_id_display() {
        let id = SubmissionId::from("a1b2c3d".to_string());
        assert_eq!(format!("{}", id), "a1b2c3d");
    }

    #[test]
    fn submission_id_equality_and_hashing() {
        let id1 = SubmissionId::from("a1b2c3d".to_string());
        let id2 = SubmissionId::from("a1b2c3d".to_string());
        let id3 = SubmissionId::from("b3c4d5e".to_string());

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        let mut set = HashSet::new();
        set.insert(id1.clone());
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }

    // --- SubmissionId content_addressed tests (ADR 081) ---

    #[test]
    fn content_addressed_deterministic() {
        let id1 = SubmissionId::content_addressed(b"hello", "ses-1", "");
        let id2 = SubmissionId::content_addressed(b"hello", "ses-1", "");
        assert_eq!(id1, id2);
    }

    #[test]
    fn content_addressed_different_payloads() {
        let id1 = SubmissionId::content_addressed(b"hello", "ses-1", "");
        let id2 = SubmissionId::content_addressed(b"world", "ses-1", "");
        assert_ne!(id1, id2);
    }

    #[test]
    fn content_addressed_different_sessions() {
        let id1 = SubmissionId::content_addressed(b"hello", "ses-1", "");
        let id2 = SubmissionId::content_addressed(b"hello", "ses-2", "");
        assert_ne!(id1, id2);
    }

    #[test]
    fn content_addressed_chains() {
        let first = SubmissionId::content_addressed(b"hello", "ses-1", "");
        let second = SubmissionId::content_addressed(b"hello", "ses-1", first.as_str());
        assert_ne!(first, second, "parent submission must change the hash");
    }

    #[test]
    fn content_addressed_is_64_char_hex() {
        let id = SubmissionId::content_addressed(b"test", "ses-1", "");
        assert_eq!(
            id.as_str().len(),
            64,
            "SHA-256 produces 64 hex chars: {}",
            id
        );
        assert!(
            id.as_str().chars().all(|c| c.is_ascii_hexdigit()),
            "should be hex: {}",
            id
        );
    }

    // --- SessionId tests ---

    #[test]
    fn session_id_generates_unique_ids() {
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn session_id_has_ses_prefix() {
        let id = SessionId::new();
        assert!(id.as_str().starts_with("ses-"));
        assert!(id.to_string().starts_with("ses-"));
    }

    #[test]
    fn session_id_equality_and_hashing() {
        let id1 = SessionId::from("ses-abc123".to_string());
        let id2 = SessionId::from("ses-abc123".to_string());
        let id3 = SessionId::from("ses-def456".to_string());

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        let mut set = HashSet::new();
        set.insert(id1.clone());
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }

    #[test]
    fn session_id_from_string() {
        let id = SessionId::from("ses-custom".to_string());
        assert_eq!(id.as_str(), "ses-custom");
    }

    #[test]
    fn session_id_display() {
        let id = SessionId::from("ses-abc123".to_string());
        assert_eq!(format!("{}", id), "ses-abc123");
    }

    // --- TimelineId tests (ADR 093) ---

    #[test]
    fn timeline_id_main_is_one() {
        let id = TimelineId::main();
        assert_eq!(id.as_str(), "1");
    }

    #[test]
    fn timeline_id_display() {
        let id = TimelineId::main();
        assert_eq!(format!("{}", id), "1");
    }

    #[test]
    fn timeline_id_from_string() {
        let id = TimelineId::from("42".to_string());
        assert_eq!(id.as_str(), "42");
    }

    #[test]
    fn timeline_id_from_i64() {
        let id = TimelineId::from(7i64);
        assert_eq!(id.as_str(), "7");
    }

    #[test]
    fn timeline_id_equality_and_hashing() {
        let id1 = TimelineId::from("3".to_string());
        let id2 = TimelineId::from("3".to_string());
        let id3 = TimelineId::from("5".to_string());

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        let mut set = HashSet::new();
        set.insert(id1.clone());
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }

    // --- Sequence tests ---

    #[test]
    fn sequence_first_is_one() {
        let seq = Sequence::first();
        assert_eq!(seq.as_u32(), 1);
    }

    #[test]
    fn sequence_next_increments() {
        let seq1 = Sequence::first();
        let seq2 = seq1.next();
        let seq3 = seq2.next();

        assert_eq!(seq1.as_u32(), 1);
        assert_eq!(seq2.as_u32(), 2);
        assert_eq!(seq3.as_u32(), 3);
    }

    #[test]
    fn sequence_display_format() {
        let seq = Sequence::from(42);
        assert_eq!(format!("{}", seq), "42");
    }

    #[test]
    fn sequence_from_u32() {
        let seq = Sequence::from(5);
        assert_eq!(seq.as_u32(), 5);
    }

    #[test]
    fn sequence_equality() {
        let seq1 = Sequence::from(3);
        let seq2 = Sequence::from(3);
        let seq3 = Sequence::from(4);

        assert_eq!(seq1, seq2);
        assert_ne!(seq1, seq3);
    }

    // --- HarnessType tests ---

    #[test]
    fn harness_type_as_str() {
        assert_eq!(HarnessType::Cli.as_str(), "cli");
        assert_eq!(HarnessType::Web.as_str(), "web");
        assert_eq!(HarnessType::Api.as_str(), "api");
        assert_eq!(HarnessType::Whatsapp.as_str(), "whatsapp");
        assert_eq!(HarnessType::Grpc.as_str(), "grpc");
    }

    #[test]
    fn harness_type_from_str_round_trips() {
        let types = [
            HarnessType::Cli,
            HarnessType::Web,
            HarnessType::Api,
            HarnessType::Whatsapp,
            HarnessType::Grpc,
        ];
        for ht in types {
            assert_eq!(HarnessType::from_str(ht.as_str()), Ok(ht));
        }
    }

    #[test]
    fn harness_type_from_str_unknown_returns_none() {
        assert!(HarnessType::from_str("unknown").is_err());
        assert!(HarnessType::from_str("").is_err());
    }

    #[test]
    fn harness_type_display() {
        assert_eq!(format!("{}", HarnessType::Cli), "cli");
        assert_eq!(format!("{}", HarnessType::Web), "web");
        assert_eq!(format!("{}", HarnessType::Api), "api");
        assert_eq!(format!("{}", HarnessType::Whatsapp), "whatsapp");
        assert_eq!(format!("{}", HarnessType::Grpc), "grpc");
    }

    #[test]
    fn harness_type_equality() {
        assert_eq!(HarnessType::Cli, HarnessType::Cli);
        assert_ne!(HarnessType::Cli, HarnessType::Web);
    }

    #[test]
    fn harness_type_is_copy() {
        let id = HarnessType::Cli;
        let id2 = id;
        assert_eq!(id, id2);
    }

    #[test]
    fn harness_type_hashable() {
        let mut set = HashSet::new();
        set.insert(HarnessType::Cli);
        set.insert(HarnessType::Web);

        assert!(set.contains(&HarnessType::Cli));
        assert!(set.contains(&HarnessType::Web));
        assert!(!set.contains(&HarnessType::Api));
    }
}
