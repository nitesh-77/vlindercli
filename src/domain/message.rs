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

use super::RuntimeType;
use super::operation::Operation;
use super::routing_key::{AgentId, ServiceBackend};
use super::service_payloads::{RequestPayload, ResponsePayload};
use super::diagnostics::{
    InvokeDiagnostics, RequestDiagnostics, ServiceDiagnostics,
    ContainerDiagnostics, DelegateDiagnostics,
};

/// Protocol version stamped on every message at construction time.
///
/// Any new message type MUST include a `protocol_version: String` field
/// and set it to `PROTOCOL_VERSION.to_string()` in its constructor.
pub const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- Supporting types ---

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
        use sha2::{Sha256, Digest};
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
#[derive(Clone, Debug, Serialize)]
pub struct InvokeMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: TimelineId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub harness: HarnessType,
    pub runtime: RuntimeType,
    pub agent_id: AgentId,
    #[serde(skip)]
    pub payload: Vec<u8>,
    /// Initial state hash from the previous turn's State trailer (ADR 055).
    /// None for the first invocation or when state tracking is not active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the harness (ADR 071).
    #[serde(skip)]
    pub diagnostics: InvokeDiagnostics,
}

impl InvokeMessage {
    pub fn new(
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent_id: AgentId,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: InvokeDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline,
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
#[derive(Clone, Debug, Serialize)]
pub struct RequestMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: TimelineId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_id: AgentId,
    pub service: ServiceBackend,
    pub operation: Operation,
    pub sequence: Sequence,
    #[serde(skip)]
    pub payload: RequestPayload,
    /// State hash at the time this request was made (ADR 055).
    /// Records the state context before the service processes the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the bridge (ADR 071).
    #[serde(skip)]
    pub diagnostics: RequestDiagnostics,
}

impl RequestMessage {
    pub fn new(
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: RequestDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline,
            submission,
            session,
            agent_id,
            service,
            operation,
            sequence,
            payload: RequestPayload::Legacy(payload),
            state,
            diagnostics,
        }
    }
}

/// Response message: Service → Runtime
///
/// Service responds to a request, echoing all dimensions for traceability.
#[derive(Clone, Debug, Serialize)]
pub struct ResponseMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: TimelineId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_id: AgentId,
    pub service: ServiceBackend,
    pub operation: Operation,
    pub sequence: Sequence,
    #[serde(skip)]
    pub payload: ResponsePayload,
    pub correlation_id: MessageId,
    /// State hash after this response (ADR 055).
    /// Present when the operation changed state (e.g. kv-put returns new hash).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the service worker (ADR 071).
    #[serde(skip)]
    pub diagnostics: ServiceDiagnostics,
}

impl ResponseMessage {
    /// Create a response from a request, echoing all dimensions.
    ///
    /// Uses placeholder diagnostics. Call `from_request_with_diagnostics()`
    /// when the service worker has real metrics.
    pub fn from_request(request: &RequestMessage, payload: Vec<u8>) -> Self {
        let placeholder = ServiceDiagnostics::storage(
            request.service.service_type(), request.service.backend_str(),
            request.operation, 0, 0,
        );
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
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline: request.timeline.clone(),
            submission: request.submission.clone(),
            session: request.session.clone(),
            agent_id: request.agent_id.clone(),
            service: request.service,
            operation: request.operation,
            sequence: request.sequence,
            payload: ResponsePayload::Legacy(payload),
            correlation_id: request.id.clone(),
            state: None,
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
#[derive(Clone, Debug, Serialize)]
pub struct DelegateMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: TimelineId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub caller: AgentId,
    pub target: AgentId,
    #[serde(skip)]
    pub payload: Vec<u8>,
    /// The "handle" — caller polls this for result.
    pub reply_subject: String,
    /// Caller's state hash at the time of delegation (ADR 055).
    /// Records the delegating agent's state context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the container runtime (ADR 071).
    #[serde(skip)]
    pub diagnostics: DelegateDiagnostics,
}

impl DelegateMessage {
    pub fn new(
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        caller: AgentId,
        target: AgentId,
        payload: Vec<u8>,
        reply_subject: impl Into<String>,
        state: Option<String>,
        diagnostics: DelegateDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline,
            submission,
            session,
            caller,
            target,
            payload,
            reply_subject: reply_subject.into(),
            state,
            diagnostics,
        }
    }
}

/// Complete message: Runtime → Harness
///
/// Signals that a submission has finished.
#[derive(Clone, Debug, Serialize)]
pub struct CompleteMessage {
    pub id: MessageId,
    pub protocol_version: String,
    pub timeline: TimelineId,
    pub submission: SubmissionId,
    pub session: SessionId,
    pub agent_id: AgentId,
    pub harness: HarnessType,
    #[serde(skip)]
    pub payload: Vec<u8>,
    /// Final state hash after this invocation (ADR 055).
    /// None when state tracking is not active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Diagnostics from the container runtime (ADR 071).
    #[serde(skip)]
    pub diagnostics: ContainerDiagnostics,
}

impl CompleteMessage {
    pub fn new(
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        harness: HarnessType,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: ContainerDiagnostics,
    ) -> Self {
        Self {
            id: MessageId::new(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            timeline,
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
/// separately via `create_reply_with_state()` after reading from QueueBridge.
/// Uses placeholder ContainerDiagnostics — the runtime replaces with real diagnostics.
impl ExpectsReply for InvokeMessage {
    type Reply = CompleteMessage;

    fn create_reply(&self, payload: Vec<u8>) -> CompleteMessage {
        CompleteMessage::new(
            self.timeline.clone(),
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
    /// QueueBridge after the agent finishes.
    pub fn create_reply_with_state(&self, payload: Vec<u8>, state: Option<String>) -> CompleteMessage {
        CompleteMessage::new(
            self.timeline.clone(),
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
    /// Used by the runtime when it has both the final state from QueueBridge
    /// and real container metadata from Podman.
    pub fn create_reply_with_diagnostics(
        &self,
        payload: Vec<u8>,
        state: Option<String>,
        diagnostics: ContainerDiagnostics,
    ) -> CompleteMessage {
        CompleteMessage::new(
            self.timeline.clone(),
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
    /// Get the protocol version.
    pub fn protocol_version(&self) -> &str {
        match self {
            ObservableMessage::Invoke(m) => &m.protocol_version,
            ObservableMessage::Request(m) => &m.protocol_version,
            ObservableMessage::Response(m) => &m.protocol_version,
            ObservableMessage::Complete(m) => &m.protocol_version,
            ObservableMessage::Delegate(m) => &m.protocol_version,
        }
    }

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

    /// Get the timeline ID (ADR 093: branch-scoped subjects).
    pub fn timeline(&self) -> &TimelineId {
        match self {
            ObservableMessage::Invoke(m) => &m.timeline,
            ObservableMessage::Request(m) => &m.timeline,
            ObservableMessage::Response(m) => &m.timeline,
            ObservableMessage::Complete(m) => &m.timeline,
            ObservableMessage::Delegate(m) => &m.timeline,
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

    /// Get the payload as raw bytes.
    ///
    /// For Request and Response messages, delegates to `legacy_bytes()` on
    /// the typed payload enum. For all other message types, returns the
    /// raw `Vec<u8>` directly.
    pub fn payload(&self) -> &[u8] {
        match self {
            ObservableMessage::Invoke(m) => &m.payload,
            ObservableMessage::Request(m) => m.payload.legacy_bytes(),
            ObservableMessage::Response(m) => m.payload.legacy_bytes(),
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
    use super::super::storage::ObjectStorageType;
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
        assert_eq!(id.as_str().len(), 64, "SHA-256 produces 64 hex chars: {}", id);
        assert!(id.as_str().chars().all(|c| c.is_ascii_hexdigit()),
            "should be hex: {}", id);
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

    fn test_agent_id() -> AgentId {
        AgentId::new("echo-agent")
    }

    #[test]
    fn invoke_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = InvokeMessage::new(
            TimelineId::main(),
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
            TimelineId::main(),
            submission.clone(),
            session.clone(),
            agent_id.clone(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::first(),
            b"key".to_vec(),
            None,
            test_request_diag(),
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.agent_id, agent_id);
        assert_eq!(msg.service, ServiceBackend::Kv(ObjectStorageType::Sqlite));
        assert_eq!(msg.operation, Operation::Get);
        assert_eq!(msg.sequence.as_u32(), 1);
        assert_eq!(msg.payload.legacy_bytes(), b"key");
    }

    #[test]
    fn response_from_request_echoes_dimensions() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let request = RequestMessage::new(
            TimelineId::main(),
            submission.clone(),
            session.clone(),
            agent_id.clone(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::from(3),
            b"key".to_vec(),
            None,
            test_request_diag(),
        );

        let response = ResponseMessage::from_request(&request, b"value".to_vec());

        // Response echoes all request dimensions
        assert_eq!(response.submission, request.submission);
        assert_eq!(response.session, request.session);
        assert_eq!(response.agent_id, request.agent_id);
        assert_eq!(response.service, request.service);
        assert_eq!(response.operation, request.operation);
        assert_eq!(response.sequence, request.sequence);

        // But has its own ID and the request's ID as correlation
        assert_ne!(response.id, request.id);
        assert_eq!(response.correlation_id, request.id);

        // And its own payload
        assert_eq!(response.payload.legacy_bytes(), b"value");
    }

    #[test]
    fn complete_message_creation() {
        let submission = test_submission();
        let session = SessionId::new();
        let agent_id = test_agent_id();
        let msg = CompleteMessage::new(
            TimelineId::main(),
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
            TimelineId::main(),
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
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::first(),
            b"key".to_vec(),
            None,
            test_request_diag(),
        );

        // create_reply returns ResponseMessage (compiler enforced)
        let response: ResponseMessage = request.create_reply(b"value".to_vec());

        // Dimensions are echoed from request
        assert_eq!(response.submission, request.submission);
        assert_eq!(response.session, request.session);
        assert_eq!(response.agent_id, request.agent_id);
        assert_eq!(response.service, request.service);
        assert_eq!(response.operation, request.operation);
        assert_eq!(response.sequence, request.sequence);
        // Correlation links reply to original
        assert_eq!(response.correlation_id, request.id);
        // Payload is the reply payload
        assert_eq!(response.payload.legacy_bytes(), b"value");
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
            TimelineId::main(),
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
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::first(),
            b"test".to_vec(),
            None,
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
            TimelineId::main(),
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
            TimelineId::main(),
            submission.clone(),
            session.clone(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"summarize this".to_vec(),
            "vlinder.sub.delegate-reply.coordinator.summarizer.abc123",
            None,
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );

        assert_eq!(msg.submission, submission);
        assert_eq!(msg.session, session);
        assert_eq!(msg.caller, AgentId::new("coordinator"));
        assert_eq!(msg.target, AgentId::new("summarizer"));
        assert_eq!(msg.payload, b"summarize this");
        assert!(msg.reply_subject.contains("delegate-reply"));
    }

    #[test]
    fn observable_message_from_delegate() {
        let submission = test_submission();
        let delegate = DelegateMessage::new(
            TimelineId::main(),
            submission.clone(),
            SessionId::new(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            b"test".to_vec(),
            "reply.subject",
            None,
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
