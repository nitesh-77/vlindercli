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
}
