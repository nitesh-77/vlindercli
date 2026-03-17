//! ObservableMessage — unified enum wrapping all typed messages.
//!
//! `ObservableMessageHeaders` carries all message metadata without the payload.
//! Transports deserialize their wire format into headers; the domain assembles
//! the final `ObservableMessage` by attaching the payload via `assemble()`.

use super::super::dag::MessageType;
use super::super::diagnostics::{
    DelegateDiagnostics, InvokeDiagnostics, RequestDiagnostics, RuntimeDiagnostics,
    ServiceDiagnostics,
};
use super::super::operation::Operation;
use super::super::routing_key::{AgentId, Nonce, ServiceBackend};
use super::super::RuntimeType;
use super::complete::CompleteMessage;
use super::delegate::DelegateMessage;
use super::fork::ForkMessage;
use super::identity::{
    BranchId, DagNodeId, HarnessType, MessageId, Sequence, SessionId, SubmissionId,
};
use super::invoke::InvokeMessage;
use super::repair::RepairMessage;
use super::request::RequestMessage;
use super::response::ResponseMessage;

/// Unified message type wrapping all typed messages.
///
/// Used for polymorphic handling when the specific message type isn't known
/// at compile time (e.g., receiving from a queue).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ObservableMessage {
    Invoke(InvokeMessage),
    Request(RequestMessage),
    Response(ResponseMessage),
    Complete(CompleteMessage),
    Delegate(DelegateMessage),
    Repair(RepairMessage),
    Fork(ForkMessage),
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
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent_id: AgentId,
        state: Option<String>,
        diagnostics: InvokeDiagnostics,
        dag_parent: DagNodeId,
    },
    Request {
        id: MessageId,
        protocol_version: String,
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
        state: Option<String>,
        diagnostics: RequestDiagnostics,
        checkpoint: Option<String>,
    },
    Response {
        id: MessageId,
        protocol_version: String,
        branch: BranchId,
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
        checkpoint: Option<String>,
    },
    Complete {
        id: MessageId,
        protocol_version: String,
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        harness: HarnessType,
        state: Option<String>,
        diagnostics: RuntimeDiagnostics,
    },
    Delegate {
        id: MessageId,
        protocol_version: String,
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        caller: AgentId,
        target: AgentId,
        nonce: Nonce,
        state: Option<String>,
        diagnostics: DelegateDiagnostics,
    },
    Repair {
        id: MessageId,
        protocol_version: String,
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_id: AgentId,
        harness: HarnessType,
        dag_parent: DagNodeId,
        checkpoint: String,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
        state: Option<String>,
    },
    Fork {
        id: MessageId,
        protocol_version: String,
        branch: BranchId,
        submission: SubmissionId,
        session: SessionId,
        agent_name: String,
        branch_name: String,
        fork_point: DagNodeId,
    },
}

impl ObservableMessageHeaders {
    /// Assemble a full `ObservableMessage` by attaching a payload.
    pub fn assemble(self, payload: Vec<u8>) -> ObservableMessage {
        match self {
            Self::Invoke {
                id,
                protocol_version,
                branch,
                submission,
                session,
                harness,
                runtime,
                agent_id,
                state,
                diagnostics,
                dag_parent,
            } => ObservableMessage::Invoke(InvokeMessage {
                id,
                protocol_version,
                branch,
                submission,
                session,
                harness,
                runtime,
                agent_id,
                payload,
                state,
                diagnostics,
                dag_parent,
            }),
            Self::Request {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_id,
                service,
                operation,
                sequence,
                state,
                diagnostics,
                checkpoint,
            } => ObservableMessage::Request(RequestMessage {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_id,
                service,
                operation,
                sequence,
                payload,
                state,
                diagnostics,
                checkpoint,
            }),
            Self::Response {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_id,
                service,
                operation,
                sequence,
                correlation_id,
                state,
                diagnostics,
                status_code,
                checkpoint,
            } => ObservableMessage::Response(ResponseMessage {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_id,
                service,
                operation,
                sequence,
                payload,
                correlation_id,
                state,
                diagnostics,
                status_code,
                checkpoint,
            }),
            Self::Complete {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_id,
                harness,
                state,
                diagnostics,
            } => ObservableMessage::Complete(CompleteMessage {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_id,
                harness,
                payload,
                state,
                diagnostics,
            }),
            Self::Delegate {
                id,
                protocol_version,
                branch,
                submission,
                session,
                caller,
                target,
                nonce,
                state,
                diagnostics,
            } => ObservableMessage::Delegate(DelegateMessage {
                id,
                protocol_version,
                branch,
                submission,
                session,
                caller,
                target,
                payload,
                nonce,
                state,
                diagnostics,
            }),
            Self::Repair {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_id,
                harness,
                dag_parent,
                checkpoint,
                service,
                operation,
                sequence,
                state,
            } => ObservableMessage::Repair(RepairMessage {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_id,
                harness,
                dag_parent,
                checkpoint,
                service,
                operation,
                sequence,
                payload,
                state,
            }),
            Self::Fork {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_name,
                branch_name,
                fork_point,
            } => ObservableMessage::Fork(ForkMessage {
                id,
                protocol_version,
                branch,
                submission,
                session,
                agent_name,
                branch_name,
                fork_point,
            }),
        }
    }
}

impl ObservableMessage {
    pub fn message_type(&self) -> MessageType {
        match self {
            ObservableMessage::Invoke(_) => MessageType::Invoke,
            ObservableMessage::Request(_) => MessageType::Request,
            ObservableMessage::Response(_) => MessageType::Response,
            ObservableMessage::Complete(_) => MessageType::Complete,
            ObservableMessage::Delegate(_) => MessageType::Delegate,
            ObservableMessage::Repair(_) => MessageType::Repair,
            ObservableMessage::Fork(_) => MessageType::Fork,
        }
    }

    pub fn protocol_version(&self) -> &str {
        match self {
            ObservableMessage::Invoke(m) => &m.protocol_version,
            ObservableMessage::Request(m) => &m.protocol_version,
            ObservableMessage::Response(m) => &m.protocol_version,
            ObservableMessage::Complete(m) => &m.protocol_version,
            ObservableMessage::Delegate(m) => &m.protocol_version,
            ObservableMessage::Repair(m) => &m.protocol_version,
            ObservableMessage::Fork(m) => &m.protocol_version,
        }
    }

    pub fn id(&self) -> &MessageId {
        match self {
            ObservableMessage::Invoke(m) => &m.id,
            ObservableMessage::Request(m) => &m.id,
            ObservableMessage::Response(m) => &m.id,
            ObservableMessage::Complete(m) => &m.id,
            ObservableMessage::Delegate(m) => &m.id,
            ObservableMessage::Repair(m) => &m.id,
            ObservableMessage::Fork(m) => &m.id,
        }
    }

    pub fn submission(&self) -> &SubmissionId {
        match self {
            ObservableMessage::Invoke(m) => &m.submission,
            ObservableMessage::Request(m) => &m.submission,
            ObservableMessage::Response(m) => &m.submission,
            ObservableMessage::Complete(m) => &m.submission,
            ObservableMessage::Delegate(m) => &m.submission,
            ObservableMessage::Repair(m) => &m.submission,
            ObservableMessage::Fork(m) => &m.submission,
        }
    }

    pub fn branch(&self) -> &BranchId {
        match self {
            ObservableMessage::Invoke(m) => &m.branch,
            ObservableMessage::Request(m) => &m.branch,
            ObservableMessage::Response(m) => &m.branch,
            ObservableMessage::Complete(m) => &m.branch,
            ObservableMessage::Delegate(m) => &m.branch,
            ObservableMessage::Repair(m) => &m.branch,
            ObservableMessage::Fork(m) => &m.branch,
        }
    }

    pub fn session(&self) -> &SessionId {
        match self {
            ObservableMessage::Invoke(m) => &m.session,
            ObservableMessage::Request(m) => &m.session,
            ObservableMessage::Response(m) => &m.session,
            ObservableMessage::Complete(m) => &m.session,
            ObservableMessage::Delegate(m) => &m.session,
            ObservableMessage::Repair(m) => &m.session,
            ObservableMessage::Fork(m) => &m.session,
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
            ObservableMessage::Repair(m) => &m.payload,
            ObservableMessage::Fork(_) => &[],
        }
    }

    /// Restore payload from a separate storage column.
    ///
    /// Needed because payload is `#[serde(skip)]` on some message types,
    /// so it's lost during JSON round-trip through `message_blob`.
    pub fn set_payload(&mut self, payload: Vec<u8>) {
        match self {
            ObservableMessage::Invoke(m) => m.payload = payload,
            ObservableMessage::Request(m) => m.payload = payload,
            ObservableMessage::Response(m) => m.payload = payload,
            ObservableMessage::Complete(m) => m.payload = payload,
            ObservableMessage::Delegate(m) => m.payload = payload,
            ObservableMessage::Repair(m) => m.payload = payload,
            ObservableMessage::Fork(_) => {}
        }
    }

    /// Extract (from, to) routing pair.
    pub fn from_to(&self) -> (String, String) {
        match self {
            ObservableMessage::Invoke(m) => {
                (m.harness.as_str().to_string(), m.agent_id.to_string())
            }
            ObservableMessage::Request(m) => (
                m.agent_id.to_string(),
                format!("{}.{}", m.service.service_type(), m.service.backend_str()),
            ),
            ObservableMessage::Response(m) => (
                format!("{}.{}", m.service.service_type(), m.service.backend_str()),
                m.agent_id.to_string(),
            ),
            ObservableMessage::Complete(m) => {
                (m.agent_id.to_string(), m.harness.as_str().to_string())
            }
            ObservableMessage::Delegate(m) => (m.caller.to_string(), m.target.to_string()),
            ObservableMessage::Repair(m) => {
                (m.harness.as_str().to_string(), m.agent_id.to_string())
            }
            ObservableMessage::Fork(m) => ("platform".to_string(), m.agent_name.clone()),
        }
    }

    /// State hash (ADR 055).
    pub fn state(&self) -> Option<&str> {
        match self {
            ObservableMessage::Invoke(m) => m.state.as_deref(),
            ObservableMessage::Request(m) => m.state.as_deref(),
            ObservableMessage::Response(m) => m.state.as_deref(),
            ObservableMessage::Complete(m) => m.state.as_deref(),
            ObservableMessage::Delegate(m) => m.state.as_deref(),
            ObservableMessage::Repair(m) => m.state.as_deref(),
            ObservableMessage::Fork(_) => None,
        }
    }

    /// Checkpoint handler name (ADR 111).
    pub fn checkpoint(&self) -> Option<&str> {
        match self {
            ObservableMessage::Request(m) => m.checkpoint.as_deref(),
            ObservableMessage::Response(m) => m.checkpoint.as_deref(),
            ObservableMessage::Repair(m) => Some(&m.checkpoint),
            _ => None,
        }
    }

    /// Operation name (ADR 113).
    pub fn operation(&self) -> Option<&str> {
        match self {
            ObservableMessage::Request(m) => Some(m.operation.as_str()),
            ObservableMessage::Response(m) => Some(m.operation.as_str()),
            ObservableMessage::Repair(m) => Some(m.operation.as_str()),
            _ => None,
        }
    }

    /// Serialize diagnostics to JSON bytes.
    pub fn diagnostics_json(&self) -> Vec<u8> {
        let json = match self {
            ObservableMessage::Invoke(m) => serde_json::to_vec(&m.diagnostics),
            ObservableMessage::Request(m) => serde_json::to_vec(&m.diagnostics),
            ObservableMessage::Response(m) => serde_json::to_vec(&m.diagnostics),
            ObservableMessage::Complete(m) => serde_json::to_vec(&m.diagnostics),
            ObservableMessage::Delegate(m) => serde_json::to_vec(&m.diagnostics),
            ObservableMessage::Repair(_) | ObservableMessage::Fork(_) => return Vec::new(),
        };
        json.unwrap_or_default()
    }

    /// Raw stderr from container execution (Complete/Delegate only).
    pub fn stderr(&self) -> &[u8] {
        match self {
            ObservableMessage::Complete(m) => &m.diagnostics.stderr,
            ObservableMessage::Delegate(m) => &m.diagnostics.runtime.stderr,
            _ => &[],
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

impl From<RepairMessage> for ObservableMessage {
    fn from(msg: RepairMessage) -> Self {
        ObservableMessage::Repair(msg)
    }
}

impl From<ForkMessage> for ObservableMessage {
    fn from(msg: ForkMessage) -> Self {
        ObservableMessage::Fork(msg)
    }
}
