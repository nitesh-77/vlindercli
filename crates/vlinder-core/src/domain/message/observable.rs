//! ObservableMessage — unified enum wrapping all typed messages.

use super::identity::{MessageId, SubmissionId, SessionId, TimelineId};
use super::invoke::InvokeMessage;
use super::request::RequestMessage;
use super::response::ResponseMessage;
use super::complete::CompleteMessage;
use super::delegate::DelegateMessage;

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
