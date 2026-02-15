//! Message queue trait definition (ADR 044).
//!
//! Typed message methods for send and receive:
//! - `send_invoke()` / `receive_invoke()`: Harness → Runtime
//! - `send_request()` / `receive_request()`: Runtime → Service
//! - `send_response()` / `receive_response()`: Service → Runtime
//! - `send_complete()` / `receive_complete()`: Runtime → Harness
//!
//! Each receive method returns a tuple of (TypedMessage, AckFn) where
//! AckFn acknowledges successful processing.

use super::{Agent, CompleteMessage, DelegateMessage, InvokeMessage, Operation, RequestMessage, ResponseMessage, ResourceId, ServiceType, SubmissionId};
use std::fmt;

// --- MessageQueue Trait ---

/// A message queue for sending and receiving typed messages (ADR 044).
pub trait MessageQueue {
    // -------------------------------------------------------------------------
    // Routing helpers - build queue names for backend-specific routing (ADR 043)
    // -------------------------------------------------------------------------

    /// Build queue name for a service call with backend type.
    ///
    /// Used by workers to subscribe and by agents to send service requests.
    ///
    /// # Arguments
    /// - `service`: Service type (Kv, Vec, Infer, Embed)
    /// - `backend`: Backend implementation (e.g., "sqlite", "ollama", "memory")
    /// - `action`: Operation (e.g., Get, Put, Search)
    fn service_queue(&self, service: ServiceType, backend: &str, action: Operation) -> String;

    /// Build queue name for an agent with runtime type.
    ///
    /// Used by harness to send to agents and by runtimes to receive.
    ///
    /// # Arguments
    /// - `runtime`: Runtime type (e.g., "wasm", "docker")
    /// - `agent`: The agent to build queue name for
    fn agent_queue(&self, runtime: &str, agent: &Agent) -> String;

    // -------------------------------------------------------------------------
    // Typed message methods (ADR 044)
    // -------------------------------------------------------------------------

    /// Send an InvokeMessage (Harness → Runtime).
    ///
    /// Implementation determines routing from message dimensions.
    fn send_invoke(&self, msg: InvokeMessage) -> Result<(), QueueError>;

    /// Send a RequestMessage (Runtime → Service).
    ///
    /// Implementation determines routing from message dimensions.
    fn send_request(&self, msg: RequestMessage) -> Result<(), QueueError>;

    /// Send a ResponseMessage (Service → Runtime).
    ///
    /// Implementation determines routing from message dimensions.
    fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError>;

    /// Send a CompleteMessage (Runtime → Harness).
    ///
    /// Implementation determines routing from message dimensions.
    fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError>;

    // -------------------------------------------------------------------------
    // Typed receive methods (ADR 044)
    // -------------------------------------------------------------------------

    /// Receive an InvokeMessage from a subject pattern.
    ///
    /// Returns the typed message with all dimensions intact.
    fn receive_invoke(&self, subject_pattern: &str) -> Result<(InvokeMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;

    /// Receive a RequestMessage for a service/backend/operation pattern.
    ///
    /// Used by workers to receive typed service requests.
    /// Returns the typed message with all dimensions intact.
    fn receive_request(&self, service: ServiceType, backend: &str, operation: Operation) -> Result<(RequestMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;

    /// Receive a ResponseMessage for the given request.
    ///
    /// The queue builds the filter pattern from the request's dimensions.
    /// Returns the typed message with all dimensions intact.
    fn receive_response(&self, request: &RequestMessage) -> Result<(ResponseMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;

    /// Receive a CompleteMessage for a specific submission.
    ///
    /// Each invocation polls its own submission-scoped consumer (ADR 052).
    fn receive_complete(&self, submission: &SubmissionId, harness: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;

    // -------------------------------------------------------------------------
    // Delegation methods (ADR 056)
    // -------------------------------------------------------------------------

    /// Create an opaque reply address for a delegation.
    ///
    /// The caller stores this in DelegateMessage and later passes it to
    /// receive_complete_on_subject() to poll for the result.
    /// The format is queue-implementation-specific.
    fn create_reply_address(&self, submission: &SubmissionId, caller: &str, target: &str) -> String;

    /// Send a DelegateMessage (Agent → Agent via runtime).
    fn send_delegate(&self, msg: DelegateMessage) -> Result<(), QueueError>;

    /// Receive a DelegateMessage for a target agent.
    fn receive_delegate(&self, target_agent: &str) -> Result<(DelegateMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;

    /// Send a CompleteMessage to a specific subject (for delegation replies).
    fn send_complete_to_subject(&self, msg: CompleteMessage, subject: &str) -> Result<(), QueueError>;

    /// Receive a CompleteMessage on a specific subject (for delegation replies).
    fn receive_complete_on_subject(&self, subject: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;
}

// --- Errors ---

#[derive(Debug)]
pub enum QueueError {
    /// Queue does not exist
    QueueNotFound(String),
    /// Failed to send message
    SendFailed(String),
    /// Failed to receive message
    ReceiveFailed(String),
    /// Receive timed out without a message (not an error, just no work)
    Timeout,
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueError::QueueNotFound(q) => write!(f, "queue not found: {}", q),
            QueueError::SendFailed(msg) => write!(f, "send failed: {}", msg),
            QueueError::ReceiveFailed(msg) => write!(f, "receive failed: {}", msg),
            QueueError::Timeout => write!(f, "receive timed out"),
        }
    }
}

impl std::error::Error for QueueError {}

// --- Routing ---

/// Extract the agent name from a registry-assigned ResourceId.
///
/// Registry IDs have the format `<registry>/agents/<name>`.
/// The last path component is the agent name, used as the NATS subject token.
pub fn agent_routing_key(agent_id: &ResourceId) -> String {
    if let Some(path) = agent_id.path() {
        if let Some(name) = path.rsplit('/').next() {
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    agent_id.as_str().to_string()
}
