//! Message types for queue communication (ADR 044).
//!
//! Typed messages with explicit fields for observability:
//! - `InvokeMessage`: Harness → Runtime (start a submission)
//! - `RequestMessage`: Runtime → Service (agent calls a service)
//! - `ResponseMessage`: Service → Runtime (service replies)
//! - `CompleteMessage`: Runtime → Harness (submission finished)

use std::fmt;
use uuid::Uuid;

use crate::domain::{ResourceId, RuntimeType};

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

// ============================================================================
// Typed Messages (ADR 044)
// ============================================================================

/// Trait for messages that expect a reply.
///
/// The associated type `Reply` specifies what message type is expected
/// as a response. Use `create_reply()` to construct the reply - it ensures
/// the correct type and echoes all relevant dimensions.
///
/// Terminal messages (Complete, Response) don't implement this trait
/// because they ARE replies.
pub trait ExpectsReply {
    type Reply;

    /// Create the reply for this message (recommended way to construct replies).
    ///
    /// Ensures the correct reply type and echoes all dimensions from the original.
    fn create_reply(&self, payload: Vec<u8>) -> Self::Reply;
}

/// Invoke message: Harness → Runtime
///
/// Starts a submission by invoking an agent.
/// Expects a CompleteMessage in response (enforced by ExpectsReply trait).
#[derive(Clone, Debug)]
pub struct InvokeMessage {
    pub id: MessageId,
    pub submission: SubmissionId,
    pub harness: HarnessType,
    pub runtime: RuntimeType,
    pub agent_id: ResourceId,
    pub payload: Vec<u8>,
}

impl InvokeMessage {
    pub fn new(
        submission: SubmissionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent_id: ResourceId,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            submission,
            harness,
            runtime,
            agent_id,
            payload,
        }
    }
}

/// Request message: Runtime → Service
///
/// Agent requests a service operation (kv, vec, infer, embed).
/// Expects a ResponseMessage in response (enforced by ExpectsReply trait).
#[derive(Clone, Debug)]
pub struct RequestMessage {
    pub id: MessageId,
    pub submission: SubmissionId,
    pub agent_id: ResourceId,
    pub service: String,
    pub backend: String,
    pub operation: String,
    pub sequence: Sequence,
    pub payload: Vec<u8>,
}

impl RequestMessage {
    pub fn new(
        submission: SubmissionId,
        agent_id: ResourceId,
        service: impl Into<String>,
        backend: impl Into<String>,
        operation: impl Into<String>,
        sequence: Sequence,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            submission,
            agent_id,
            service: service.into(),
            backend: backend.into(),
            operation: operation.into(),
            sequence,
            payload,
        }
    }
}

/// Response message: Service → Runtime
///
/// Service responds to a request, echoing all dimensions for traceability.
#[derive(Clone, Debug)]
pub struct ResponseMessage {
    pub id: MessageId,
    pub submission: SubmissionId,
    pub agent_id: ResourceId,
    pub service: String,
    pub backend: String,
    pub operation: String,
    pub sequence: Sequence,
    pub payload: Vec<u8>,
    pub correlation_id: MessageId,
}

impl ResponseMessage {
    /// Create a response from a request, echoing all dimensions.
    pub fn from_request(request: &RequestMessage, payload: Vec<u8>) -> Self {
        Self {
            id: MessageId::new(),
            submission: request.submission.clone(),
            agent_id: request.agent_id.clone(),
            service: request.service.clone(),
            backend: request.backend.clone(),
            operation: request.operation.clone(),
            sequence: request.sequence,
            payload,
            correlation_id: request.id.clone(),
        }
    }
}

/// Complete message: Runtime → Harness
///
/// Signals that a submission has finished.
#[derive(Clone, Debug)]
pub struct CompleteMessage {
    pub id: MessageId,
    pub submission: SubmissionId,
    pub agent_id: ResourceId,
    pub harness: HarnessType,
    pub payload: Vec<u8>,
    pub correlation_id: MessageId,
}

impl CompleteMessage {
    pub fn new(
        submission: SubmissionId,
        agent_id: ResourceId,
        harness: HarnessType,
        payload: Vec<u8>,
        correlation_id: MessageId,
    ) -> Self {
        Self {
            id: MessageId::new(),
            submission,
            agent_id,
            harness,
            payload,
            correlation_id,
        }
    }
}

// ============================================================================
// ExpectsReply trait implementations
// ============================================================================

/// InvokeMessage expects CompleteMessage as its reply.
impl ExpectsReply for InvokeMessage {
    type Reply = CompleteMessage;

    fn create_reply(&self, payload: Vec<u8>) -> CompleteMessage {
        CompleteMessage::new(
            self.submission.clone(),
            self.agent_id.clone(),
            self.harness,
            payload,
            self.id.clone(),
        )
    }
}

/// RequestMessage expects ResponseMessage as its reply.
impl ExpectsReply for RequestMessage {
    type Reply = ResponseMessage;

    fn create_reply(&self, payload: Vec<u8>) -> ResponseMessage {
        ResponseMessage::from_request(self, payload)
    }
}

// Note: CompleteMessage and ResponseMessage do NOT implement ExpectsReply.
// They are terminal messages - attempting to get their Reply type is a
// compile-time error.

// ============================================================================
// ObservableMessage enum
// ============================================================================

/// Unified message type wrapping all typed messages.
///
/// Used for polymorphic handling when the specific message type isn't known
/// at compile time (e.g., receiving from a queue).
#[derive(Clone, Debug)]
pub enum ObservableMessage {
    Invoke(InvokeMessage),
    Request(RequestMessage),
    Response(ResponseMessage),
    Complete(CompleteMessage),
}

impl ObservableMessage {
    /// Get the message ID.
    pub fn id(&self) -> &MessageId {
        match self {
            ObservableMessage::Invoke(m) => &m.id,
            ObservableMessage::Request(m) => &m.id,
            ObservableMessage::Response(m) => &m.id,
            ObservableMessage::Complete(m) => &m.id,
        }
    }

    /// Get the submission ID.
    pub fn submission(&self) -> &SubmissionId {
        match self {
            ObservableMessage::Invoke(m) => &m.submission,
            ObservableMessage::Request(m) => &m.submission,
            ObservableMessage::Response(m) => &m.submission,
            ObservableMessage::Complete(m) => &m.submission,
        }
    }

    /// Get the agent ID.
    pub fn agent_id(&self) -> &ResourceId {
        match self {
            ObservableMessage::Invoke(m) => &m.agent_id,
            ObservableMessage::Request(m) => &m.agent_id,
            ObservableMessage::Response(m) => &m.agent_id,
            ObservableMessage::Complete(m) => &m.agent_id,
        }
    }

    /// Get the payload.
    pub fn payload(&self) -> &[u8] {
        match self {
            ObservableMessage::Invoke(m) => &m.payload,
            ObservableMessage::Request(m) => &m.payload,
            ObservableMessage::Response(m) => &m.payload,
            ObservableMessage::Complete(m) => &m.payload,
        }
    }
}

impl From<InvokeMessage> for ObservableMessage {
    fn from(msg: InvokeMessage) -> Self {
        ObservableMessage::Invoke(msg)
    }
}

impl From<RequestMessage> for ObservableMessage {
    fn from(msg: RequestMessage) -> Self {
        ObservableMessage::Request(msg)
    }
}

impl From<ResponseMessage> for ObservableMessage {
    fn from(msg: ResponseMessage) -> Self {
        ObservableMessage::Response(msg)
    }
}

impl From<CompleteMessage> for ObservableMessage {
    fn from(msg: CompleteMessage) -> Self {
        ObservableMessage::Complete(msg)
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

    // --- Typed message tests ---

    fn test_agent_id() -> ResourceId {
        ResourceId::new("file:///test/echo-agent.wasm")
    }

    #[test]
    fn invoke_message_creation() {
        let submission = SubmissionId::new();
        let agent_id = test_agent_id();
        let msg = InvokeMessage::new(
            submission.clone(),
            HarnessType::Cli,
            RuntimeType::Wasm,
            agent_id.clone(),
            b"hello".to_vec(),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.harness, HarnessType::Cli);
        assert_eq!(msg.runtime, RuntimeType::Wasm);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.payload, b"hello");
    }

    #[test]
    fn request_message_creation() {
        let submission = SubmissionId::new();
        let agent_id = test_agent_id();
        let msg = RequestMessage::new(
            submission.clone(),
            agent_id.clone(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.service, "kv");
        assert_eq!(msg.backend, "sqlite");
        assert_eq!(msg.operation, "get");
        assert_eq!(msg.sequence.as_u32(), 1);
        assert_eq!(msg.payload, b"key");
    }

    #[test]
    fn response_from_request_echoes_dimensions() {
        let submission = SubmissionId::new();
        let agent_id = test_agent_id();
        let request = RequestMessage::new(
            submission.clone(),
            agent_id.clone(),
            "kv",
            "sqlite",
            "get",
            Sequence::from(3),
            b"key".to_vec(),
        );

        let response = ResponseMessage::from_request(&request, b"value".to_vec());

        // Response echoes all request dimensions
        assert_eq!(response.submission, request.submission);
        assert_eq!(response.agent_id, request.agent_id);
        assert_eq!(response.service, request.service);
        assert_eq!(response.backend, request.backend);
        assert_eq!(response.operation, request.operation);
        assert_eq!(response.sequence, request.sequence);

        // But has its own ID and the request's ID as correlation
        assert_ne!(response.id, request.id);
        assert_eq!(response.correlation_id, request.id);

        // And its own payload
        assert_eq!(response.payload, b"value");
    }

    #[test]
    fn complete_message_creation() {
        let submission = SubmissionId::new();
        let agent_id = test_agent_id();
        let invoke_id = MessageId::new();
        let msg = CompleteMessage::new(
            submission.clone(),
            agent_id.clone(),
            HarnessType::Cli,
            b"result".to_vec(),
            invoke_id.clone(),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.harness, HarnessType::Cli);
        assert_eq!(msg.payload, b"result");
        assert_eq!(msg.correlation_id, invoke_id);
    }

    // --- ExpectsReply trait tests ---

    #[test]
    fn invoke_create_reply_returns_complete() {
        let invoke = InvokeMessage::new(
            SubmissionId::new(),
            HarnessType::Cli,
            RuntimeType::Wasm,
            test_agent_id(),
            b"input".to_vec(),
        );

        // create_reply returns CompleteMessage (compiler enforced)
        let complete: CompleteMessage = invoke.create_reply(b"output".to_vec());

        // Dimensions are echoed from invoke
        assert_eq!(complete.submission, invoke.submission);
        assert_eq!(complete.agent_id, invoke.agent_id);
        assert_eq!(complete.harness, invoke.harness);
        // Correlation links reply to original
        assert_eq!(complete.correlation_id, invoke.id);
        // Payload is the reply payload
        assert_eq!(complete.payload, b"output");
    }

    #[test]
    fn request_create_reply_returns_response() {
        let request = RequestMessage::new(
            SubmissionId::new(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
        );

        // create_reply returns ResponseMessage (compiler enforced)
        let response: ResponseMessage = request.create_reply(b"value".to_vec());

        // Dimensions are echoed from request
        assert_eq!(response.submission, request.submission);
        assert_eq!(response.agent_id, request.agent_id);
        assert_eq!(response.service, request.service);
        assert_eq!(response.backend, request.backend);
        assert_eq!(response.operation, request.operation);
        assert_eq!(response.sequence, request.sequence);
        // Correlation links reply to original
        assert_eq!(response.correlation_id, request.id);
        // Payload is the reply payload
        assert_eq!(response.payload, b"value");
    }

    // Note: These would NOT compile (uncomment to verify):
    //
    // fn try_create_reply_from_complete() {
    //     let complete = CompleteMessage::new(...);
    //     complete.create_reply(...);  // ERROR: CompleteMessage doesn't implement ExpectsReply
    // }

    // --- ObservableMessage tests ---

    #[test]
    fn observable_message_from_invoke() {
        let invoke = InvokeMessage::new(
            SubmissionId::new(),
            HarnessType::Cli,
            RuntimeType::Wasm,
            test_agent_id(),
            b"test".to_vec(),
        );
        let id = invoke.id.clone();
        let submission = invoke.submission.clone();

        let observable: ObservableMessage = invoke.into();

        assert_eq!(observable.id(), &id);
        assert_eq!(observable.submission(), &submission);
        matches!(observable, ObservableMessage::Invoke(_));
    }

    #[test]
    fn observable_message_from_request() {
        let request = RequestMessage::new(
            SubmissionId::new(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"test".to_vec(),
        );
        let id = request.id.clone();

        let observable: ObservableMessage = request.into();

        assert_eq!(observable.id(), &id);
        matches!(observable, ObservableMessage::Request(_));
    }

    #[test]
    fn observable_message_common_accessors() {
        let submission = SubmissionId::new();
        let agent_id = test_agent_id();
        let invoke = InvokeMessage::new(
            submission.clone(),
            HarnessType::Cli,
            RuntimeType::Wasm,
            agent_id.clone(),
            b"payload".to_vec(),
        );

        let observable: ObservableMessage = invoke.into();

        // All common fields accessible through enum
        assert_eq!(observable.submission(), &submission);
        assert_eq!(observable.agent_id(), &agent_id);
        assert_eq!(observable.payload(), b"payload");
    }
}
