//! Queue trait definition (ADR 044).
//!
//! Typed message methods for send and receive:
//! - `send_invoke()` / `receive_invoke()`: Harness → Runtime
//! - `send_request()` / `receive_request()`: Runtime → Service
//! - `send_response()` / `receive_response()`: Service → Runtime
//! - `send_complete()` / `receive_complete()`: Runtime → Harness
//!
//! Each receive method returns a tuple of (TypedMessage, AckFn) where
//! AckFn acknowledges successful processing.

use super::{CompleteMessage, InvokeMessage, RequestMessage, ResponseMessage};
use crate::domain::Agent;
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
    /// - `service`: Service type (e.g., "kv", "vec", "infer", "embed")
    /// - `backend`: Backend implementation (e.g., "sqlite", "ollama", "memory")
    /// - `action`: Operation (e.g., "get", "put", "search") - empty for bare service
    ///
    /// # Examples
    /// - `service_queue("kv", "sqlite", "get")` → queue name for SQLite kv-get
    /// - `service_queue("infer", "ollama", "")` → queue name for Ollama inference
    fn service_queue(&self, service: &str, backend: &str, action: &str) -> String;

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
    fn receive_request(&self, service: &str, backend: &str, operation: &str) -> Result<(RequestMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;

    /// Receive a ResponseMessage for the given request.
    ///
    /// The queue builds the filter pattern from the request's dimensions.
    /// Returns the typed message with all dimensions intact.
    fn receive_response(&self, request: &RequestMessage) -> Result<(ResponseMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;

    /// Receive a CompleteMessage for a harness.
    ///
    /// Used by harness to receive job completion notifications.
    fn receive_complete(&self, harness_pattern: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError>;
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
