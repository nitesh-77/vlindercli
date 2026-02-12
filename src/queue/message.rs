//! Message types for queue communication (ADR 044).
//!
//! Typed messages with explicit fields for observability:
//! - `InvokeMessage`: Harness → Runtime (start a submission)
//! - `RequestMessage`: Runtime → Service (agent calls a service)
//! - `ResponseMessage`: Service → Runtime (service replies)
//! - `CompleteMessage`: Runtime → Harness (submission finished)

use std::fmt;
use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::domain::{ResourceId, RuntimeType};
use super::diagnostics::{
    InvokeDiagnostics, RequestDiagnostics, ServiceDiagnostics,
    ContainerDiagnostics, DelegateDiagnostics,
};

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
    /// In session mode, SubmissionIds are git commit SHAs derived from
    /// the conversation store. This factory exists for backwards compatibility
    /// and will be removed once all paths use session-derived IDs.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
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
    pub session: SessionId,
    pub harness: HarnessType,
    pub runtime: RuntimeType,
    pub agent_id: ResourceId,
    pub payload: Vec<u8>,
    /// Initial state hash from the previous turn's State trailer (ADR 055).
    /// None for the first invocation or when state tracking is not active.
    pub state: Option<String>,
    /// Diagnostics from the harness (ADR 071).
    pub diagnostics: InvokeDiagnostics,
}

impl InvokeMessage {
    pub fn new(
        submission: SubmissionId,
        session: SessionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent_id: ResourceId,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: InvokeDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            submission,
            session,
            harness,
            runtime,
            agent_id,
            payload,
            state,
            diagnostics,
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
    pub session: SessionId,
    pub agent_id: ResourceId,
    pub service: String,
    pub backend: String,
    pub operation: String,
    pub sequence: Sequence,
    pub payload: Vec<u8>,
    /// Diagnostics from the bridge (ADR 071).
    pub diagnostics: RequestDiagnostics,
}

impl RequestMessage {
    pub fn new(
        submission: SubmissionId,
        session: SessionId,
        agent_id: ResourceId,
        service: impl Into<String>,
        backend: impl Into<String>,
        operation: impl Into<String>,
        sequence: Sequence,
        payload: Vec<u8>,
        diagnostics: RequestDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            submission,
            session,
            agent_id,
            service: service.into(),
            backend: backend.into(),
            operation: operation.into(),
            sequence,
            payload,
            diagnostics,
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
    pub session: SessionId,
    pub agent_id: ResourceId,
    pub service: String,
    pub backend: String,
    pub operation: String,
    pub sequence: Sequence,
    pub payload: Vec<u8>,
    pub correlation_id: MessageId,
    /// Diagnostics from the service worker (ADR 071).
    pub diagnostics: ServiceDiagnostics,
}

impl ResponseMessage {
    /// Create a response from a request, echoing all dimensions.
    ///
    /// Uses placeholder diagnostics. Call `from_request_with_diagnostics()`
    /// when the service worker has real metrics.
    pub fn from_request(request: &RequestMessage, payload: Vec<u8>) -> Self {
        let placeholder = ServiceDiagnostics::storage("unknown", "unknown", "unknown", 0, 0);
        Self::from_request_with_diagnostics(request, payload, placeholder)
    }

    /// Create a response with real diagnostics from the service worker.
    pub fn from_request_with_diagnostics(
        request: &RequestMessage,
        payload: Vec<u8>,
        diagnostics: ServiceDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            submission: request.submission.clone(),
            session: request.session.clone(),
            agent_id: request.agent_id.clone(),
            service: request.service.clone(),
            backend: request.backend.clone(),
            operation: request.operation.clone(),
            sequence: request.sequence,
            payload,
            correlation_id: request.id.clone(),
            diagnostics,
        }
    }
}

/// Delegate message: Agent → Agent (via runtime)
///
/// One agent invoking another. The platform routes through the queue,
/// dispatches the target, and sends the result to the reply subject.
/// Does NOT implement ExpectsReply — the reply is managed by the runtime,
/// not the type system.
#[derive(Clone, Debug)]
pub struct DelegateMessage {
    pub id: MessageId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub caller_agent: String,
    pub target_agent: String,
    pub payload: Vec<u8>,
    /// The "handle" — caller polls this for result.
    pub reply_subject: String,
    /// Diagnostics from the container runtime (ADR 071).
    pub diagnostics: DelegateDiagnostics,
}

impl DelegateMessage {
    pub fn new(
        submission: SubmissionId,
        session: SessionId,
        caller_agent: impl Into<String>,
        target_agent: impl Into<String>,
        payload: Vec<u8>,
        reply_subject: impl Into<String>,
        diagnostics: DelegateDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            submission,
            session,
            caller_agent: caller_agent.into(),
            target_agent: target_agent.into(),
            payload,
            reply_subject: reply_subject.into(),
            diagnostics,
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
    pub session: SessionId,
    pub agent_id: ResourceId,
    pub harness: HarnessType,
    pub payload: Vec<u8>,
    /// Final state hash after this invocation (ADR 055).
    /// None when state tracking is not active.
    pub state: Option<String>,
    /// Diagnostics from the container runtime (ADR 071).
    pub diagnostics: ContainerDiagnostics,
}

impl CompleteMessage {
    pub fn new(
        submission: SubmissionId,
        session: SessionId,
        agent_id: ResourceId,
        harness: HarnessType,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: ContainerDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            submission,
            session,
            agent_id,
            harness,
            payload,
            state,
            diagnostics,
        }
    }
}

// ============================================================================
// ExpectsReply trait implementations
// ============================================================================

/// InvokeMessage expects CompleteMessage as its reply.
///
/// Note: `create_reply()` does NOT carry state — the runtime sets state
/// separately via `create_reply_with_state()` after reading from HttpBridge.
/// Uses placeholder ContainerDiagnostics — the runtime replaces with real diagnostics.
impl ExpectsReply for InvokeMessage {
    type Reply = CompleteMessage;

    fn create_reply(&self, payload: Vec<u8>) -> CompleteMessage {
        CompleteMessage::new(
            self.submission.clone(),
            self.session.clone(),
            self.agent_id.clone(),
            self.harness,
            payload,
            None,
            ContainerDiagnostics::placeholder(0),
        )
    }
}

impl InvokeMessage {
    /// Create a reply with the final state hash (ADR 055).
    ///
    /// Used by the runtime when it has read the final state from the
    /// HttpBridge after the agent finishes.
    pub fn create_reply_with_state(&self, payload: Vec<u8>, state: Option<String>) -> CompleteMessage {
        CompleteMessage::new(
            self.submission.clone(),
            self.session.clone(),
            self.agent_id.clone(),
            self.harness,
            payload,
            state,
            ContainerDiagnostics::placeholder(0),
        )
    }

    /// Create a reply with state and real container diagnostics (ADR 073).
    ///
    /// Used by the runtime when it has both the final state from HttpBridge
    /// and real container metadata from Podman.
    pub fn create_reply_with_diagnostics(
        &self,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: ContainerDiagnostics,
    ) -> CompleteMessage {
        CompleteMessage::new(
            self.submission.clone(),
            self.session.clone(),
            self.agent_id.clone(),
            self.harness,
            payload,
            state,
            diagnostics,
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
    Delegate(DelegateMessage),
}

impl ObservableMessage {
    /// Get the message ID.
    pub fn id(&self) -> &MessageId {
        match self {
            ObservableMessage::Invoke(m) => &m.id,
            ObservableMessage::Request(m) => &m.id,
            ObservableMessage::Response(m) => &m.id,
            ObservableMessage::Complete(m) => &m.id,
            ObservableMessage::Delegate(m) => &m.id,
        }
    }

    /// Get the submission ID.
    pub fn submission(&self) -> &SubmissionId {
        match self {
            ObservableMessage::Invoke(m) => &m.submission,
            ObservableMessage::Request(m) => &m.submission,
            ObservableMessage::Response(m) => &m.submission,
            ObservableMessage::Complete(m) => &m.submission,
            ObservableMessage::Delegate(m) => &m.submission,
        }
    }

    /// Get the session ID (ADR 071: all 5 types carry session).
    pub fn session(&self) -> &SessionId {
        match self {
            ObservableMessage::Invoke(m) => &m.session,
            ObservableMessage::Request(m) => &m.session,
            ObservableMessage::Response(m) => &m.session,
            ObservableMessage::Complete(m) => &m.session,
            ObservableMessage::Delegate(m) => &m.session,
        }
    }

    /// Get the payload.
    pub fn payload(&self) -> &[u8] {
        match self {
            ObservableMessage::Invoke(m) => &m.payload,
            ObservableMessage::Request(m) => &m.payload,
            ObservableMessage::Response(m) => &m.payload,
            ObservableMessage::Complete(m) => &m.payload,
            ObservableMessage::Delegate(m) => &m.payload,
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

impl From<DelegateMessage> for ObservableMessage {
    fn from(msg: DelegateMessage) -> Self {
        ObservableMessage::Delegate(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn test_submission() -> SubmissionId {
        SubmissionId::from("a1b2c3d".to_string())
    }

    fn test_invoke_diag() -> InvokeDiagnostics {
        InvokeDiagnostics {
            harness_version: "0.1.0".to_string(),
            history_turns: 0,
        }
    }

    fn test_request_diag() -> RequestDiagnostics {
        RequestDiagnostics {
            sequence: 1,
            endpoint: "/test".to_string(),
            request_bytes: 0,
            received_at_ms: 0,
        }
    }

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
        ResourceId::new("http://127.0.0.1:9000/agents/echo-agent")
    }

    #[test]
    fn invoke_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = InvokeMessage::new(
            submission.clone(),
            session.clone(),
            HarnessType::Cli,
            RuntimeType::Container,
            agent_id.clone(),
            b"hello".to_vec(),
            None,
            test_invoke_diag(),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.harness, HarnessType::Cli);
        assert_eq!(msg.runtime, RuntimeType::Container);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.payload, b"hello");
        assert_eq!(msg.state, None);
    }

    #[test]
    fn request_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = RequestMessage::new(
            submission.clone(),
            session.clone(),
            agent_id.clone(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
            test_request_diag(),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.service, "kv");
        assert_eq!(msg.backend, "sqlite");
        assert_eq!(msg.operation, "get");
        assert_eq!(msg.sequence.as_u32(), 1);
        assert_eq!(msg.payload, b"key");
    }

    #[test]
    fn response_from_request_echoes_dimensions() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let request = RequestMessage::new(
            submission.clone(),
            session.clone(),
            agent_id.clone(),
            "kv",
            "sqlite",
            "get",
            Sequence::from(3),
            b"key".to_vec(),
            test_request_diag(),
        );

        let response = ResponseMessage::from_request(&request, b"value".to_vec());

        // Response echoes all request dimensions
        assert_eq!(response.submission, request.submission);
        assert_eq!(response.session, request.session);
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
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = CompleteMessage::new(
            submission.clone(),
            session.clone(),
            agent_id.clone(),
            HarnessType::Cli,
            b"result".to_vec(),
            None,
            ContainerDiagnostics::placeholder(0),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.harness, HarnessType::Cli);
        assert_eq!(msg.payload, b"result");
        assert_eq!(msg.state, None);
    }

    // --- ExpectsReply trait tests ---

    #[test]
    fn invoke_create_reply_returns_complete() {
        let invoke = InvokeMessage::new(
            test_submission(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"input".to_vec(),
            None,
            test_invoke_diag(),
        );

        // create_reply returns CompleteMessage (compiler enforced)
        let complete: CompleteMessage = invoke.create_reply(b"output".to_vec());

        // Dimensions are echoed from invoke
        assert_eq!(complete.submission, invoke.submission);
        assert_eq!(complete.session, invoke.session);
        assert_eq!(complete.agent_id, invoke.agent_id);
        assert_eq!(complete.harness, invoke.harness);
        // Payload is the reply payload
        assert_eq!(complete.payload, b"output");
    }

    #[test]
    fn request_create_reply_returns_response() {
        let request = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
            test_request_diag(),
        );

        // create_reply returns ResponseMessage (compiler enforced)
        let response: ResponseMessage = request.create_reply(b"value".to_vec());

        // Dimensions are echoed from request
        assert_eq!(response.submission, request.submission);
        assert_eq!(response.session, request.session);
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
            test_submission(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"test".to_vec(),
            None,
            test_invoke_diag(),
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
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"test".to_vec(),
            test_request_diag(),
        );
        let id = request.id.clone();

        let observable: ObservableMessage = request.into();

        assert_eq!(observable.id(), &id);
        matches!(observable, ObservableMessage::Request(_));
    }

    #[test]
    fn observable_message_common_accessors() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let invoke = InvokeMessage::new(
            submission.clone(),
            session.clone(),
            HarnessType::Cli,
            RuntimeType::Container,
            agent_id.clone(),
            b"payload".to_vec(),
            None,
            test_invoke_diag(),
        );

        let observable: ObservableMessage = invoke.into();

        // All common fields accessible through enum
        assert_eq!(observable.submission(), &submission);
        assert_eq!(observable.session(), &session);
        assert_eq!(observable.payload(), b"payload");
    }

    // --- DelegateMessage tests ---

    #[test]
    fn delegate_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let msg = DelegateMessage::new(
            submission.clone(),
            session.clone(),
            "coordinator",
            "summarizer",
            b"summarize this".to_vec(),
            "vlinder.sub.delegate-reply.coordinator.summarizer.abc123",
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.caller_agent, "coordinator");
        assert_eq!(msg.target_agent, "summarizer");
        assert_eq!(msg.payload, b"summarize this");
        assert!(msg.reply_subject.contains("delegate-reply"));
    }

    #[test]
    fn observable_message_from_delegate() {
        let submission = test_submission();
        let delegate = DelegateMessage::new(
            submission.clone(),
            SessionId::new(),
            "coordinator",
            "summarizer",
            b"test".to_vec(),
            "reply.subject",
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );
        let id = delegate.id.clone();

        let observable: ObservableMessage = delegate.into();

        assert_eq!(observable.id(), &id);
        assert_eq!(observable.submission(), &submission);
        assert_eq!(observable.payload(), b"test");
        assert!(matches!(observable, ObservableMessage::Delegate(_)));
    }
}
