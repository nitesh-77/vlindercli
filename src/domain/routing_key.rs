//! Routing keys as domain types (ADR 096).
//!
//! A routing key captures the dimensions that uniquely identify where a
//! message should be delivered. Collision-freedom is structural — derived
//! from equality over all dimensions.

use super::{
    HarnessType, ObjectStorageType, Operation, RuntimeType, Sequence, SubmissionId, TimelineId,
    VectorStorageType,
};

/// Agent identity within the routing bounded context.
///
/// Distinct from ResourceId (registration context). For routing, this is
/// the unique identifier that distinguishes one agent from another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentId(String);

impl AgentId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Inference backend implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InferenceBackendType {
    Ollama,
    OpenRouter,
}

/// Embedding backend implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmbeddingBackendType {
    Ollama,
}

/// Service-backend pair for routing.
///
/// Each service type scopes its own set of valid backends. Invalid
/// combinations (e.g., Kv + Ollama) are unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceBackend {
    Kv(ObjectStorageType),
    Vec(VectorStorageType),
    Infer(InferenceBackendType),
    Embed(EmbeddingBackendType),
}

/// A value used exactly once to ensure uniqueness.
///
/// Prevents routing key collisions when the same caller delegates to the
/// same target multiple times within a submission.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Nonce(String);

impl Nonce {
    /// Generate a unique nonce.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Create a nonce from a known value (for testing or deserialization).
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Nonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Routing key for message delivery.
///
/// Each variant corresponds to one message direction in the protocol.
/// Equality and hashing derive from all fields — two routing keys are
/// equal if and only if they are the same variant and every dimension
/// matches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RoutingKey {
    Invoke {
        timeline: TimelineId,
        submission: SubmissionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent: AgentId,
    },
    Complete {
        timeline: TimelineId,
        submission: SubmissionId,
        agent: AgentId,
        harness: HarnessType,
    },
    Request {
        timeline: TimelineId,
        submission: SubmissionId,
        agent: AgentId,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
    },
    Response {
        timeline: TimelineId,
        submission: SubmissionId,
        service: ServiceBackend,
        agent: AgentId,
        operation: Operation,
        sequence: Sequence,
    },
    Delegate {
        timeline: TimelineId,
        submission: SubmissionId,
        caller: AgentId,
        target: AgentId,
    },
    DelegateReply {
        timeline: TimelineId,
        submission: SubmissionId,
        caller: AgentId,
        target: AgentId,
        nonce: Nonce,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline() -> TimelineId {
        TimelineId::main()
    }

    fn timeline_alt() -> TimelineId {
        TimelineId::from(2)
    }

    fn submission() -> SubmissionId {
        SubmissionId::from("sub-1".to_string())
    }

    fn submission_alt() -> SubmissionId {
        SubmissionId::from("sub-2".to_string())
    }

    fn agent() -> AgentId {
        AgentId::new("echo")
    }

    fn agent_alt() -> AgentId {
        AgentId::new("pensieve")
    }

    // ========================================================================
    // Collision-freedom: Invoke (5 dimensions)
    // ========================================================================

    fn base_invoke() -> RoutingKey {
        RoutingKey::Invoke {
            timeline: timeline(),
            submission: submission(),
            harness: HarnessType::Cli,
            runtime: RuntimeType::Container,
            agent: agent(),
        }
    }

    #[test]
    fn invoke_equal_when_all_dimensions_match() {
        assert_eq!(base_invoke(), base_invoke());
    }

    #[test]
    fn invoke_differs_by_timeline() {
        let mut key = base_invoke();
        if let RoutingKey::Invoke { ref mut timeline, .. } = key {
            *timeline = timeline_alt();
        }
        assert_ne!(base_invoke(), key);
    }

    #[test]
    fn invoke_differs_by_submission() {
        let mut key = base_invoke();
        if let RoutingKey::Invoke { ref mut submission, .. } = key {
            *submission = submission_alt();
        }
        assert_ne!(base_invoke(), key);
    }

    #[test]
    fn invoke_differs_by_harness() {
        let mut key = base_invoke();
        if let RoutingKey::Invoke { ref mut harness, .. } = key {
            *harness = HarnessType::Web;
        }
        assert_ne!(base_invoke(), key);
    }

    #[test]
    fn invoke_differs_by_agent() {
        let mut key = base_invoke();
        if let RoutingKey::Invoke { ref mut agent, .. } = key {
            *agent = agent_alt();
        }
        assert_ne!(base_invoke(), key);
    }

    // ========================================================================
    // Collision-freedom: Complete (4 dimensions)
    // ========================================================================

    fn base_complete() -> RoutingKey {
        RoutingKey::Complete {
            timeline: timeline(),
            submission: submission(),
            agent: agent(),
            harness: HarnessType::Cli,
        }
    }

    #[test]
    fn complete_equal_when_all_dimensions_match() {
        assert_eq!(base_complete(), base_complete());
    }

    #[test]
    fn complete_differs_by_agent() {
        let mut key = base_complete();
        if let RoutingKey::Complete { ref mut agent, .. } = key {
            *agent = agent_alt();
        }
        assert_ne!(base_complete(), key);
    }

    #[test]
    fn complete_differs_by_harness() {
        let mut key = base_complete();
        if let RoutingKey::Complete { ref mut harness, .. } = key {
            *harness = HarnessType::Web;
        }
        assert_ne!(base_complete(), key);
    }

    // ========================================================================
    // Collision-freedom: Request (6 dimensions)
    // ========================================================================

    fn base_request() -> RoutingKey {
        RoutingKey::Request {
            timeline: timeline(),
            submission: submission(),
            agent: agent(),
            service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
            operation: Operation::Get,
            sequence: Sequence::first(),
        }
    }

    #[test]
    fn request_equal_when_all_dimensions_match() {
        assert_eq!(base_request(), base_request());
    }

    #[test]
    fn request_differs_by_service_type() {
        let mut key = base_request();
        if let RoutingKey::Request { ref mut service, .. } = key {
            *service = ServiceBackend::Vec(VectorStorageType::SqliteVec);
        }
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_backend_within_service() {
        let mut key = base_request();
        if let RoutingKey::Request { ref mut service, .. } = key {
            *service = ServiceBackend::Kv(ObjectStorageType::InMemory);
        }
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_operation() {
        let mut key = base_request();
        if let RoutingKey::Request { ref mut operation, .. } = key {
            *operation = Operation::Put;
        }
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_sequence() {
        let mut key = base_request();
        if let RoutingKey::Request { ref mut sequence, .. } = key {
            *sequence = Sequence::from(2);
        }
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_agent() {
        let mut key = base_request();
        if let RoutingKey::Request { ref mut agent, .. } = key {
            *agent = agent_alt();
        }
        assert_ne!(base_request(), key);
    }

    // ========================================================================
    // Collision-freedom: Response (6 dimensions)
    // ========================================================================

    fn base_response() -> RoutingKey {
        RoutingKey::Response {
            timeline: timeline(),
            submission: submission(),
            service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
            agent: agent(),
            operation: Operation::Get,
            sequence: Sequence::first(),
        }
    }

    #[test]
    fn response_equal_when_all_dimensions_match() {
        assert_eq!(base_response(), base_response());
    }

    #[test]
    fn response_differs_by_agent() {
        let mut key = base_response();
        if let RoutingKey::Response { ref mut agent, .. } = key {
            *agent = agent_alt();
        }
        assert_ne!(base_response(), key);
    }

    #[test]
    fn response_differs_by_service() {
        let mut key = base_response();
        if let RoutingKey::Response { ref mut service, .. } = key {
            *service = ServiceBackend::Infer(InferenceBackendType::Ollama);
        }
        assert_ne!(base_response(), key);
    }

    // ========================================================================
    // Collision-freedom: Delegate (4 dimensions)
    // ========================================================================

    fn base_delegate() -> RoutingKey {
        RoutingKey::Delegate {
            timeline: timeline(),
            submission: submission(),
            caller: agent(),
            target: agent_alt(),
        }
    }

    #[test]
    fn delegate_equal_when_all_dimensions_match() {
        assert_eq!(base_delegate(), base_delegate());
    }

    #[test]
    fn delegate_differs_by_caller() {
        let mut key = base_delegate();
        if let RoutingKey::Delegate { ref mut caller, .. } = key {
            *caller = AgentId::new("planner");
        }
        assert_ne!(base_delegate(), key);
    }

    #[test]
    fn delegate_differs_by_target() {
        let mut key = base_delegate();
        if let RoutingKey::Delegate { ref mut target, .. } = key {
            *target = AgentId::new("fact-checker");
        }
        assert_ne!(base_delegate(), key);
    }

    // ========================================================================
    // Collision-freedom: DelegateReply (5 dimensions)
    // ========================================================================

    fn base_delegate_reply() -> RoutingKey {
        RoutingKey::DelegateReply {
            timeline: timeline(),
            submission: submission(),
            caller: agent(),
            target: agent_alt(),
            nonce: Nonce::new("abc123"),
        }
    }

    #[test]
    fn delegate_reply_equal_when_all_dimensions_match() {
        assert_eq!(base_delegate_reply(), base_delegate_reply());
    }

    #[test]
    fn delegate_reply_differs_by_nonce() {
        let mut key = base_delegate_reply();
        if let RoutingKey::DelegateReply { ref mut nonce, .. } = key {
            *nonce = Nonce::new("def456");
        }
        assert_ne!(base_delegate_reply(), key);
    }

    // ========================================================================
    // Cross-variant: different variants never equal
    // ========================================================================

    #[test]
    fn invoke_and_complete_never_equal() {
        assert_ne!(base_invoke(), base_complete());
    }

    #[test]
    fn request_and_response_never_equal() {
        assert_ne!(base_request(), base_response());
    }

    #[test]
    fn delegate_and_delegate_reply_never_equal() {
        assert_ne!(base_delegate(), base_delegate_reply());
    }
}
