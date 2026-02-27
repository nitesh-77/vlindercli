//! ObservableMessage — unified enum wrapping all typed messages.
//!
//! `ObservableMessageHeaders` carries all message metadata without the payload.
//! Transports deserialize their wire format into headers; the domain assembles
//! the final `ObservableMessage` by attaching the payload via `assemble()`.

use super::identity::{MessageId, SubmissionId, SessionId, TimelineId, Sequence, HarnessType};
use super::invoke::InvokeMessage;
use super::request::RequestMessage;
use super::response::ResponseMessage;
use super::complete::CompleteMessage;
use super::delegate::DelegateMessage;
use super::super::RuntimeType;
use super::super::routing_key::{AgentId, Nonce, ServiceBackend};
use super::super::operation::Operation;
use super::super::diagnostics::{
    InvokeDiagnostics, RequestDiagnostics, ServiceDiagnostics,
    ContainerDiagnostics, DelegateDiagnostics,
};

/// Unified message type wrapping all typed messages.
///
/// Used for polymorphic handling when the specific message type isn't known
/// at compile time (e.g., receiving from a queue).
#[derive(Debug)]
pub enum ObservableMessage {
    Invoke(InvokeMessage),
    Request(RequestMessage),
    Response(ResponseMessage),
    Complete(CompleteMessage),
    Delegate(DelegateMessage),
}

/// Message metadata without payload.
///
/// Transports produce this by deserializing their wire format (NATS headers,
/// SQS attributes, etc.) into typed domain fields. The domain then assembles
/// the final `ObservableMessage` by attaching the payload.
///
/// Each variant carries exactly the fields for that message type — no Options,
/// no guessing. The variant itself is the type discriminant.
#[derive(Debug)]
pub enum ObservableMessageHeaders {
    Invoke {
        id: MessageId,
        protocol_version: String,
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent_id: AgentId,
        state: Option<String>,
        diagnostics: InvokeDiagnostics,
    },
    Request {
        id: MessageId,
        protocol_version: String,
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
        state: Option<String>,
        diagnostics: RequestDiagnostics,
    },
    Response {
        id: MessageId,
        protocol_version: String,
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
        correlation_id: MessageId,
        state: Option<String>,
        diagnostics: ServiceDiagnostics,
        status_code: u16,
    },
    Complete {
        id: MessageId,
        protocol_version: String,
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        harness: HarnessType,
        state: Option<String>,
        diagnostics: ContainerDiagnostics,
    },
    Delegate {
        id: MessageId,
        protocol_version: String,
        timeline: TimelineId,
        submission: SubmissionId,
        session: SessionId,
        caller: AgentId,
        target: AgentId,
        nonce: Nonce,
        state: Option<String>,
        diagnostics: DelegateDiagnostics,
    },
}

impl ObservableMessageHeaders {
    /// Assemble a full `ObservableMessage` by attaching a payload.
    pub fn assemble(self, payload: Vec<u8>) -> ObservableMessage {
        match self {
            Self::Invoke {
                id, protocol_version, timeline, submission, session,
                harness, runtime, agent_id, state, diagnostics,
            } => ObservableMessage::Invoke(InvokeMessage {
                id, protocol_version, timeline, submission, session,
                harness, runtime, agent_id, payload, state, diagnostics,
            }),
            Self::Request {
                id, protocol_version, timeline, submission, session,
                agent_id, service, operation, sequence, state, diagnostics,
            } => ObservableMessage::Request(RequestMessage {
                id, protocol_version, timeline, submission, session,
                agent_id, service, operation, sequence, payload, state, diagnostics,
            }),
            Self::Response {
                id, protocol_version, timeline, submission, session,
                agent_id, service, operation, sequence, correlation_id,
                state, diagnostics, status_code,
            } => ObservableMessage::Response(ResponseMessage {
                id, protocol_version, timeline, submission, session,
                agent_id, service, operation, sequence, payload,
                correlation_id, state, diagnostics, status_code,
            }),
            Self::Complete {
                id, protocol_version, timeline, submission, session,
                agent_id, harness, state, diagnostics,
            } => ObservableMessage::Complete(CompleteMessage {
                id, protocol_version, timeline, submission, session,
                agent_id, harness, payload, state, diagnostics,
            }),
            Self::Delegate {
                id, protocol_version, timeline, submission, session,
                caller, target, nonce, state, diagnostics,
            } => ObservableMessage::Delegate(DelegateMessage {
                id, protocol_version, timeline, submission, session,
                caller, target, payload, nonce, state, diagnostics,
            }),
        }
    }
}

impl ObservableMessage {
    pub fn protocol_version(&self) -> &str {
        match self {
            ObservableMessage::Invoke(m) => &m.protocol_version,
            ObservableMessage::Request(m) => &m.protocol_version,
            ObservableMessage::Response(m) => &m.protocol_version,
            ObservableMessage::Complete(m) => &m.protocol_version,
            ObservableMessage::Delegate(m) => &m.protocol_version,
        }
    }

    pub fn id(&self) -> &MessageId {
        match self {
            ObservableMessage::Invoke(m) => &m.id,
            ObservableMessage::Request(m) => &m.id,
            ObservableMessage::Response(m) => &m.id,
            ObservableMessage::Complete(m) => &m.id,
            ObservableMessage::Delegate(m) => &m.id,
        }
    }

    pub fn submission(&self) -> &SubmissionId {
        match self {
            ObservableMessage::Invoke(m) => &m.submission,
            ObservableMessage::Request(m) => &m.submission,
            ObservableMessage::Response(m) => &m.submission,
            ObservableMessage::Complete(m) => &m.submission,
            ObservableMessage::Delegate(m) => &m.submission,
        }
    }

    pub fn timeline(&self) -> &TimelineId {
        match self {
            ObservableMessage::Invoke(m) => &m.timeline,
            ObservableMessage::Request(m) => &m.timeline,
            ObservableMessage::Response(m) => &m.timeline,
            ObservableMessage::Complete(m) => &m.timeline,
            ObservableMessage::Delegate(m) => &m.timeline,
        }
    }

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
    fn from(msg: InvokeMessage) -> Self { ObservableMessage::Invoke(msg) }
}

impl From<RequestMessage> for ObservableMessage {
    fn from(msg: RequestMessage) -> Self { ObservableMessage::Request(msg) }
}

impl From<ResponseMessage> for ObservableMessage {
    fn from(msg: ResponseMessage) -> Self { ObservableMessage::Response(msg) }
}

impl From<CompleteMessage> for ObservableMessage {
    fn from(msg: CompleteMessage) -> Self { ObservableMessage::Complete(msg) }
}

impl From<DelegateMessage> for ObservableMessage {
    fn from(msg: DelegateMessage) -> Self { ObservableMessage::Delegate(msg) }
}
