//! Message queue trait definition (ADR 044).
//!
//! Typed message methods for send and receive:
//! - `send_invoke()` / `receive_invoke()`: Harness → Runtime (ADR 121)
//! - `send_request()` / `receive_request()`: Runtime → Service
//! - `send_response()` / `receive_response()`: Service → Runtime
//! - `send_complete()` / `receive_complete()`: Runtime → Harness
//!
//! Each receive method returns a tuple of (`TypedMessage`, `AckFn`) where
//! `AckFn` acknowledges successful processing.

use super::{
    AgentName, CompleteMessage, DataRoutingKey, DelegateMessage, DelegateReplyMessage, ForkMessage,
    HarnessType, InvokeMessage, Operation, PromoteMessage, RepairMessage, RequestMessage,
    RequestMessageV2, ResourceId, ResponseMessage, RoutingKey, ServiceBackend, SubmissionId,
};
use std::fmt;

/// One-shot closure that acknowledges a received message was processed.
pub type Acknowledgement = Box<dyn FnOnce() -> Result<(), QueueError> + Send>;

// --- MessageQueue Trait ---

/// A message queue for sending and receiving typed messages (ADR 044).
pub trait MessageQueue {
    // -------------------------------------------------------------------------
    // Invoke (ADR 121 — data plane)
    // -------------------------------------------------------------------------

    /// Send an invoke on the data plane (ADR 121).
    ///
    /// Routing key and payload are separate: the key goes into the subject,
    /// the payload goes into the NATS message body.
    fn send_invoke(&self, key: DataRoutingKey, msg: InvokeMessage) -> Result<(), QueueError>;

    /// Receive an invoke from the data plane (ADR 121).
    ///
    /// Returns the routing key, payload, and acknowledgement.
    fn receive_invoke(
        &self,
        agent: &AgentName,
    ) -> Result<(DataRoutingKey, InvokeMessage, Acknowledgement), QueueError>;

    /// Send a `RequestMessage` (Runtime → Service).
    ///
    /// Implementation determines routing from message dimensions.
    fn send_request(&self, msg: RequestMessage) -> Result<(), QueueError>;

    /// Send a `ResponseMessage` (Service → Runtime).
    ///
    /// Implementation determines routing from message dimensions.
    fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError>;

    // -------------------------------------------------------------------------
    // Complete (ADR 121 — data plane)
    // -------------------------------------------------------------------------

    /// Send a complete on the data plane (ADR 121).
    fn send_complete(&self, _key: DataRoutingKey, _msg: CompleteMessage) -> Result<(), QueueError> {
        Err(QueueError::SendFailed(
            "send_complete not implemented".into(),
        ))
    }

    /// Receive a complete from the data plane (ADR 121).
    fn receive_complete(
        &self,
        _submission: &SubmissionId,
        _harness: HarnessType,
    ) -> Result<(DataRoutingKey, CompleteMessage, Acknowledgement), QueueError> {
        Err(QueueError::Timeout)
    }

    // -------------------------------------------------------------------------
    // Request (ADR 121 — data plane)
    // -------------------------------------------------------------------------

    /// Send a request on the data plane (ADR 121).
    fn send_request_v2(
        &self,
        _key: DataRoutingKey,
        _msg: RequestMessageV2,
    ) -> Result<(), QueueError> {
        Err(QueueError::SendFailed(
            "send_request_v2 not implemented".into(),
        ))
    }

    /// Receive a request from the data plane (ADR 121).
    fn receive_request_v2(
        &self,
        _service: ServiceBackend,
        _operation: Operation,
    ) -> Result<(DataRoutingKey, RequestMessageV2, Acknowledgement), QueueError> {
        Err(QueueError::Timeout)
    }

    // -------------------------------------------------------------------------
    // Typed receive methods (ADR 044)
    // -------------------------------------------------------------------------

    /// Receive a `RequestMessage` for a service-backend/operation pair.
    ///
    /// Used by workers to receive typed service requests.
    /// Returns the typed message with all dimensions intact.
    fn receive_request(
        &self,
        service: ServiceBackend,
        operation: Operation,
    ) -> Result<(RequestMessage, Acknowledgement), QueueError>;

    /// Receive a `ResponseMessage` for the given request.
    ///
    /// The queue builds the filter pattern from the request's dimensions.
    /// Returns the typed message with all dimensions intact.
    fn receive_response(
        &self,
        request: &RequestMessage,
    ) -> Result<(ResponseMessage, Acknowledgement), QueueError>;

    // -------------------------------------------------------------------------
    // Delegation methods (ADR 056, ADR 096 §7)
    // -------------------------------------------------------------------------

    /// Send a `DelegateMessage` (Agent → Agent via runtime).
    fn send_delegate(&self, msg: DelegateMessage) -> Result<(), QueueError>;

    /// Receive a `DelegateMessage` for a target agent.
    fn receive_delegate(
        &self,
        target: &AgentName,
    ) -> Result<(DelegateMessage, Acknowledgement), QueueError>;

    /// Send a `CompleteMessage` as a delegation reply (ADR 096 §7).
    ///
    /// Routes via `RoutingKey::DelegateReply` — the nonce ensures uniqueness
    /// when the same caller delegates to the same target multiple times.
    fn send_delegate_reply(
        &self,
        msg: DelegateReplyMessage,
        reply_key: &RoutingKey,
    ) -> Result<(), QueueError>;

    /// Receive a delegation reply (ADR 096 §7).
    ///
    /// Polls for a `CompleteMessage` at the given `DelegateReply` routing key.
    fn receive_delegate_reply(
        &self,
        reply_key: &RoutingKey,
    ) -> Result<(DelegateReplyMessage, Acknowledgement), QueueError>;

    // -------------------------------------------------------------------------
    // Repair methods (ADR 113)
    // -------------------------------------------------------------------------

    /// Send a `RepairMessage` (Platform → Sidecar).
    ///
    /// Instructs the sidecar to replay a failed service call.
    fn send_repair(&self, msg: RepairMessage) -> Result<(), QueueError>;

    /// Receive a `RepairMessage` for a specific agent.
    ///
    /// The sidecar subscribes to repair messages alongside invoke.
    fn receive_repair(
        &self,
        agent: &AgentName,
    ) -> Result<(RepairMessage, Acknowledgement), QueueError>;

    // -------------------------------------------------------------------------
    // Fork methods
    // -------------------------------------------------------------------------

    /// Send a `ForkMessage` (CLI → Platform).
    ///
    /// Creates a new timeline branch in the DAG. Both SQL and git projections
    /// react to this message.
    fn send_fork(&self, msg: ForkMessage) -> Result<(), QueueError>;

    /// Send a `PromoteMessage` (CLI → Platform).
    ///
    /// Promotes a branch to main. Both SQL and git projections react to
    /// this message.
    fn send_promote(&self, msg: PromoteMessage) -> Result<(), QueueError>;

    /// Send a `SessionStartMessage` (CLI → Platform).
    ///
    /// Creates a new conversation session and its default "main" branch.
    /// Returns the `BranchId` of the default branch so callers can use it
    /// as the `BranchId` for subsequent messages.
    fn send_session_start(
        &self,
        msg: super::SessionStartMessage,
    ) -> Result<super::BranchId, QueueError>;

    // -------------------------------------------------------------------------
    // Request-reply facades (ADR 092)
    // -------------------------------------------------------------------------

    /// Send a service request and block until the response arrives.
    ///
    /// Used by agents (via sidecar provider server) for service calls.
    fn call_service(&self, msg: RequestMessage) -> Result<ResponseMessage, QueueError> {
        let msg_for_recv = msg.clone();
        send_and_wait(
            || self.send_request(msg),
            || self.receive_response(&msg_for_recv),
        )
    }

    /// Send a repair and block until the agent completes.
    ///
    /// Used by the harness to replay a failed service call (ADR 113).
    fn repair_agent(&self, msg: RepairMessage) -> Result<CompleteMessage, QueueError> {
        let submission = msg.submission.clone();
        let harness = msg.harness;
        send_and_wait(
            || self.send_repair(msg),
            || {
                self.receive_complete(&submission, harness)
                    .map(|(_key, msg, ack)| (msg, ack))
            },
        )
    }
}

// --- Request-reply internals ---

/// Send a message and poll until the correlated reply arrives (ADR 092).
///
/// Single implementation behind `call_service()` and `repair_agent()`.
fn send_and_wait<T>(
    send: impl FnOnce() -> Result<(), QueueError>,
    receive: impl Fn() -> Result<(T, Acknowledgement), QueueError>,
) -> Result<T, QueueError> {
    send()?;
    loop {
        match receive() {
            Ok((reply, ack)) => {
                let _ = ack();
                return Ok(reply);
            }
            Err(QueueError::Timeout) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => return Err(e),
        }
    }
}

// --- Errors ---

#[derive(Debug)]
pub enum QueueError {
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
            QueueError::SendFailed(msg) => write!(f, "send failed: {msg}"),
            QueueError::ReceiveFailed(msg) => write!(f, "receive failed: {msg}"),
            QueueError::Timeout => write!(f, "receive timed out"),
        }
    }
}

impl std::error::Error for QueueError {}

// --- Routing ---

/// Extract the agent name from a registry-assigned `ResourceId`.
///
/// Registry IDs have the format `<registry>/agents/<name>`.
/// The last path component is the agent name, used as the NATS subject token.
pub fn agent_routing_key(agent_id: &ResourceId) -> AgentName {
    let name = if let Some(path) = agent_id.path() {
        path.rsplit('/')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(agent_id.as_str())
    } else {
        agent_id.as_str()
    };
    AgentName::new(name)
}
