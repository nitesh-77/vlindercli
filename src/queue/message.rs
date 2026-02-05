//! Message types for queue communication.

use std::fmt;
use uuid::Uuid;

/// A message that travels through the queue.
///
/// All messages expect a response - no fire-and-forget.
#[derive(Clone, Debug)]
pub struct Message {
    pub id: MessageId,
    pub payload: Vec<u8>,
    pub reply_to: String,
    pub correlation_id: Option<MessageId>,
}

impl Message {
    /// Create a request that expects a response.
    pub fn request(payload: Vec<u8>, reply_to: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            payload,
            reply_to: reply_to.into(),
            correlation_id: None,
        }
    }

    /// Create a response to a request.
    pub fn response(payload: Vec<u8>, reply_to: impl Into<String>, correlation_id: MessageId) -> Self {
        Self {
            id: MessageId::new(),
            payload,
            reply_to: reply_to.into(),
            correlation_id: Some(correlation_id),
        }
    }
}

// --- Supporting types ---

/// Unique identifier for a message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

// --- SubmissionId (ADR 044) ---

/// Unique identifier for a user-initiated submission.
///
/// Format: `sub-{uuid}` for easy visual identification in logs.
/// A submission groups all messages related to a single user request.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubmissionId(String);

impl SubmissionId {
    /// Create a new submission ID with "sub-" prefix.
    pub fn new() -> Self {
        Self(format!("sub-{}", Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
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

// --- Sequence (ADR 044) ---

/// Sequence number for ordering interactions within a submission.
///
/// Starts at 1 and increments for each service request.
/// Used to reconstruct the order of events when debugging.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HarnessType {
    /// Command-line interface harness
    Cli,
    /// Web API harness
    Web,
    /// REST API harness
    Api,
    /// WhatsApp integration harness
    Whatsapp,
}

impl HarnessType {
    /// Get the string representation for use in subjects.
    pub fn as_str(&self) -> &'static str {
        match self {
            HarnessType::Cli => "cli",
            HarnessType::Web => "web",
            HarnessType::Api => "api",
            HarnessType::Whatsapp => "whatsapp",
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

    #[test]
    fn submission_id_generates_unique_ids() {
        let id1 = SubmissionId::new();
        let id2 = SubmissionId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn submission_id_has_sub_prefix() {
        let id = SubmissionId::new();
        assert!(id.as_str().starts_with("sub-"));
        assert!(id.to_string().starts_with("sub-"));
    }

    #[test]
    fn submission_id_equality_and_hashing() {
        let id1 = SubmissionId::from("sub-test-123".to_string());
        let id2 = SubmissionId::from("sub-test-123".to_string());
        let id3 = SubmissionId::from("sub-test-456".to_string());

        // Equality
        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        // Hashing (can be used in HashSet/HashMap)
        let mut set = HashSet::new();
        set.insert(id1.clone());
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }

    #[test]
    fn submission_id_from_string() {
        let id = SubmissionId::from("sub-custom-id".to_string());
        assert_eq!(id.as_str(), "sub-custom-id");
    }

    #[test]
    fn submission_id_display() {
        let id = SubmissionId::from("sub-abc123".to_string());
        assert_eq!(format!("{}", id), "sub-abc123");
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
    }

    #[test]
    fn harness_type_display() {
        assert_eq!(format!("{}", HarnessType::Cli), "cli");
        assert_eq!(format!("{}", HarnessType::Web), "web");
        assert_eq!(format!("{}", HarnessType::Api), "api");
        assert_eq!(format!("{}", HarnessType::Whatsapp), "whatsapp");
    }

    #[test]
    fn harness_type_equality() {
        assert_eq!(HarnessType::Cli, HarnessType::Cli);
        assert_ne!(HarnessType::Cli, HarnessType::Web);
    }

    #[test]
    fn harness_type_is_copy() {
        let id = HarnessType::Cli;
        let id2 = id; // Copy, not move
        assert_eq!(id, id2); // Both still valid
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
