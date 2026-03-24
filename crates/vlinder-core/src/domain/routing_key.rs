//! Routing keys as domain types (ADR 096).
//!
//! A routing key captures the dimensions that uniquely identify where a
//! message should be delivered. Collision-freedom is structural — derived
//! from equality over all dimensions.

use std::str::FromStr;

use super::{
    BranchId, HarnessType, ObjectStorageType, Operation, RuntimeType, Sequence, ServiceType,
    SessionId, SubmissionId, VectorStorageType,
};

/// Agent identity within the routing bounded context.
///
/// Distinct from `ResourceId` (registration context). For routing, this is
/// the unique identifier that distinguishes one agent from another.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct AgentName(String);

impl AgentName {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Inference backend implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum InferenceBackendType {
    Ollama,
    OpenRouter,
}

impl InferenceBackendType {
    pub fn as_str(&self) -> &'static str {
        match self {
            InferenceBackendType::Ollama => "ollama",
            InferenceBackendType::OpenRouter => "openrouter",
        }
    }
}

impl std::str::FromStr for InferenceBackendType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ollama" => Ok(Self::Ollama),
            "openrouter" => Ok(Self::OpenRouter),
            _ => Err(format!("unknown inference backend: {s}")),
        }
    }
}

/// Embedding backend implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EmbeddingBackendType {
    Ollama,
}

impl EmbeddingBackendType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmbeddingBackendType::Ollama => "ollama",
        }
    }
}

impl std::str::FromStr for EmbeddingBackendType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ollama" => Ok(Self::Ollama),
            _ => Err(format!("unknown embedding backend: {s}")),
        }
    }
}

/// Service-backend pair for routing.
///
/// Each service type scopes its own set of valid backends. Invalid
/// combinations (e.g., Kv + Ollama) are unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ServiceBackend {
    Kv(ObjectStorageType),
    Vec(VectorStorageType),
    Infer(InferenceBackendType),
    Embed(EmbeddingBackendType),
}

impl ServiceBackend {
    /// The service type for this backend (wire format compatibility).
    pub fn service_type(&self) -> ServiceType {
        match self {
            ServiceBackend::Kv(_) => ServiceType::Kv,
            ServiceBackend::Vec(_) => ServiceType::Vec,
            ServiceBackend::Infer(_) => ServiceType::Infer,
            ServiceBackend::Embed(_) => ServiceType::Embed,
        }
    }

    /// The backend string for this backend (wire format compatibility).
    pub fn backend_str(&self) -> &'static str {
        match self {
            ServiceBackend::Kv(b) => b.as_str(),
            ServiceBackend::Vec(b) => b.as_str(),
            ServiceBackend::Infer(b) => b.as_str(),
            ServiceBackend::Embed(b) => b.as_str(),
        }
    }

    /// Reconstruct from separate service type and backend string.
    pub fn from_parts(service: ServiceType, backend: &str) -> Option<Self> {
        match service {
            ServiceType::Kv => match backend {
                "sqlite" => Some(Self::Kv(ObjectStorageType::Sqlite)),
                "memory" => Some(Self::Kv(ObjectStorageType::InMemory)),
                _ => None,
            },
            ServiceType::Vec => match backend {
                "sqlite-vec" => Some(Self::Vec(VectorStorageType::SqliteVec)),
                "memory" => Some(Self::Vec(VectorStorageType::InMemory)),
                _ => None,
            },
            ServiceType::Infer => InferenceBackendType::from_str(backend)
                .ok()
                .map(Self::Infer),
            ServiceType::Embed => EmbeddingBackendType::from_str(backend)
                .ok()
                .map(Self::Embed),
        }
    }
}

impl std::fmt::Display for ServiceBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.service_type(), self.backend_str())
    }
}

/// A value used exactly once to ensure uniqueness.
///
/// Prevents routing key collisions when the same caller delegates to the
/// same target multiple times within a submission.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
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
/// Composite routing key: common prefix + variant-specific suffix.
///
/// Each routing key uniquely identifies a message direction in the protocol.
/// The common prefix (session, branch, submission) maps to the NATS subject
/// prefix `vlinder.{session}.{branch}.{submission}`. The kind maps to the
/// variant-specific suffix.
///
/// Equality and hashing derive from all fields — two routing keys are
/// equal if and only if they have the same prefix and the same kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoutingKey {
    pub session: SessionId,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub kind: RoutingKind,
}

/// Variant-specific routing dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RoutingKind {
    Invoke {
        harness: HarnessType,
        runtime: RuntimeType,
        agent: AgentName,
    },
    Complete {
        agent: AgentName,
        harness: HarnessType,
    },
    Request {
        agent: AgentName,
        service: ServiceBackend,
        operation: Operation,
        sequence: Sequence,
    },
    Response {
        service: ServiceBackend,
        agent: AgentName,
        operation: Operation,
        sequence: Sequence,
    },
    Delegate {
        caller: AgentName,
        target: AgentName,
    },
    DelegateReply {
        caller: AgentName,
        target: AgentName,
        nonce: Nonce,
    },
    /// Repair: Platform → Sidecar (replay a failed service call, ADR 113).
    Repair {
        harness: HarnessType,
        agent: AgentName,
    },
    /// Fork: CLI → Platform (create a branch).
    Fork { agent_name: AgentName },
    /// Promote: CLI → Platform (promote a branch to main).
    Promote { agent_name: AgentName },
}

impl RoutingKey {
    /// Produce the reply routing key for this request routing key.
    ///
    /// Every request variant deterministically maps to its reply variant
    /// (ADR 096 §3). Delegate requires a nonce to distinguish multiple
    /// delegations to the same target within one submission.
    ///
    /// Reply variants (Complete, Response, `DelegateReply`) return None —
    /// hops are one level deep (reply asymmetry).
    pub fn reply_key(&self, nonce: Option<Nonce>) -> Option<RoutingKey> {
        let reply_kind = match &self.kind {
            RoutingKind::Invoke { harness, agent, .. } => Some(RoutingKind::Complete {
                agent: agent.clone(),
                harness: *harness,
            }),
            RoutingKind::Request {
                agent,
                service,
                operation,
                sequence,
            } => Some(RoutingKind::Response {
                service: *service,
                agent: agent.clone(),
                operation: *operation,
                sequence: *sequence,
            }),
            RoutingKind::Delegate { caller, target } => nonce.map(|n| RoutingKind::DelegateReply {
                caller: caller.clone(),
                target: target.clone(),
                nonce: n,
            }),
            RoutingKind::Complete { .. }
            | RoutingKind::Response { .. }
            | RoutingKind::DelegateReply { .. }
            | RoutingKind::Repair { .. }
            | RoutingKind::Fork { .. }
            | RoutingKind::Promote { .. } => None,
        };
        reply_kind.map(|kind| RoutingKey {
            session: self.session.clone(),
            branch: self.branch,
            submission: self.submission.clone(),
            kind,
        })
    }
}

// ============================================================================
// Data-plane routing (ADR 121)
// ============================================================================

/// Data plane routing key — agent execution messages.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DataRoutingKey {
    pub session: SessionId,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub kind: DataMessageKind,
}

/// Data plane message kinds.
///
/// New variants are added here as each message type migrates from `RoutingKind`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DataMessageKind {
    Invoke {
        harness: HarnessType,
        runtime: RuntimeType,
        agent: AgentName,
    },
    Complete {
        agent: AgentName,
        harness: HarnessType,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionId {
        SessionId::try_from("00000000-0000-4000-8000-000000000001".to_string()).unwrap()
    }

    fn session_alt() -> SessionId {
        SessionId::try_from("00000000-0000-4000-8000-000000000002".to_string()).unwrap()
    }

    fn branch() -> BranchId {
        BranchId::from(1)
    }

    fn branch_alt() -> BranchId {
        BranchId::from(2)
    }

    fn submission() -> SubmissionId {
        SubmissionId::from("sub-1".to_string())
    }

    fn submission_alt() -> SubmissionId {
        SubmissionId::from("sub-2".to_string())
    }

    fn agent() -> AgentName {
        AgentName::new("echo")
    }

    fn agent_alt() -> AgentName {
        AgentName::new("pensieve")
    }

    // ========================================================================
    // Collision-freedom: Invoke (5 dimensions)
    // ========================================================================

    fn base_invoke() -> RoutingKey {
        RoutingKey {
            session: session(),
            branch: branch(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        }
    }

    #[test]
    fn invoke_equal_when_all_dimensions_match() {
        assert_eq!(base_invoke(), base_invoke());
    }

    #[test]
    fn invoke_differs_by_session() {
        let mut key = base_invoke();
        key.session = session_alt();
        assert_ne!(base_invoke(), key);
    }

    #[test]
    fn invoke_differs_by_branch() {
        let mut key = base_invoke();
        key.branch = branch_alt();
        assert_ne!(base_invoke(), key);
    }

    #[test]
    fn invoke_differs_by_submission() {
        let mut key = base_invoke();
        key.submission = submission_alt();
        assert_ne!(base_invoke(), key);
    }

    #[test]
    fn invoke_differs_by_harness() {
        let mut key = base_invoke();
        if let RoutingKind::Invoke {
            ref mut harness, ..
        } = key.kind
        {
            *harness = HarnessType::Web;
        }
        assert_ne!(base_invoke(), key);
    }

    #[test]
    fn invoke_differs_by_agent() {
        let mut key = base_invoke();
        if let RoutingKind::Invoke { ref mut agent, .. } = key.kind {
            *agent = agent_alt();
        }
        assert_ne!(base_invoke(), key);
    }

    // ========================================================================
    // Collision-freedom: Complete (4 dimensions)
    // ========================================================================

    fn base_complete() -> RoutingKey {
        RoutingKey {
            session: session(),
            branch: branch(),
            submission: submission(),
            kind: RoutingKind::Complete {
                agent: agent(),
                harness: HarnessType::Cli,
            },
        }
    }

    #[test]
    fn complete_equal_when_all_dimensions_match() {
        assert_eq!(base_complete(), base_complete());
    }

    #[test]
    fn complete_differs_by_session() {
        let mut key = base_complete();
        key.session = session_alt();
        assert_ne!(base_complete(), key);
    }

    #[test]
    fn complete_differs_by_agent() {
        let mut key = base_complete();
        if let RoutingKind::Complete { ref mut agent, .. } = key.kind {
            *agent = agent_alt();
        }
        assert_ne!(base_complete(), key);
    }

    #[test]
    fn complete_differs_by_harness() {
        let mut key = base_complete();
        if let RoutingKind::Complete {
            ref mut harness, ..
        } = key.kind
        {
            *harness = HarnessType::Web;
        }
        assert_ne!(base_complete(), key);
    }

    // ========================================================================
    // Collision-freedom: Request (6 dimensions)
    // ========================================================================

    fn base_request() -> RoutingKey {
        RoutingKey {
            session: session(),
            branch: branch(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        }
    }

    #[test]
    fn request_equal_when_all_dimensions_match() {
        assert_eq!(base_request(), base_request());
    }

    #[test]
    fn request_differs_by_session() {
        let mut key = base_request();
        key.session = session_alt();
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_service_type() {
        let mut key = base_request();
        if let RoutingKind::Request {
            ref mut service, ..
        } = key.kind
        {
            *service = ServiceBackend::Vec(VectorStorageType::SqliteVec);
        }
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_backend_within_service() {
        let mut key = base_request();
        if let RoutingKind::Request {
            ref mut service, ..
        } = key.kind
        {
            *service = ServiceBackend::Kv(ObjectStorageType::InMemory);
        }
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_operation() {
        let mut key = base_request();
        if let RoutingKind::Request {
            ref mut operation, ..
        } = key.kind
        {
            *operation = Operation::Put;
        }
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_sequence() {
        let mut key = base_request();
        if let RoutingKind::Request {
            ref mut sequence, ..
        } = key.kind
        {
            *sequence = Sequence::from(2);
        }
        assert_ne!(base_request(), key);
    }

    #[test]
    fn request_differs_by_agent() {
        let mut key = base_request();
        if let RoutingKind::Request { ref mut agent, .. } = key.kind {
            *agent = agent_alt();
        }
        assert_ne!(base_request(), key);
    }

    // ========================================================================
    // Collision-freedom: Response (6 dimensions)
    // ========================================================================

    fn base_response() -> RoutingKey {
        RoutingKey {
            session: session(),
            branch: branch(),
            submission: submission(),
            kind: RoutingKind::Response {
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                agent: agent(),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        }
    }

    #[test]
    fn response_equal_when_all_dimensions_match() {
        assert_eq!(base_response(), base_response());
    }

    #[test]
    fn response_differs_by_session() {
        let mut key = base_response();
        key.session = session_alt();
        assert_ne!(base_response(), key);
    }

    #[test]
    fn response_differs_by_agent() {
        let mut key = base_response();
        if let RoutingKind::Response { ref mut agent, .. } = key.kind {
            *agent = agent_alt();
        }
        assert_ne!(base_response(), key);
    }

    #[test]
    fn response_differs_by_service() {
        let mut key = base_response();
        if let RoutingKind::Response {
            ref mut service, ..
        } = key.kind
        {
            *service = ServiceBackend::Infer(InferenceBackendType::Ollama);
        }
        assert_ne!(base_response(), key);
    }

    // ========================================================================
    // Collision-freedom: Delegate (4 dimensions)
    // ========================================================================

    fn base_delegate() -> RoutingKey {
        RoutingKey {
            session: session(),
            branch: branch(),
            submission: submission(),
            kind: RoutingKind::Delegate {
                caller: agent(),
                target: agent_alt(),
            },
        }
    }

    #[test]
    fn delegate_equal_when_all_dimensions_match() {
        assert_eq!(base_delegate(), base_delegate());
    }

    #[test]
    fn delegate_differs_by_session() {
        let mut key = base_delegate();
        key.session = session_alt();
        assert_ne!(base_delegate(), key);
    }

    #[test]
    fn delegate_differs_by_caller() {
        let mut key = base_delegate();
        if let RoutingKind::Delegate { ref mut caller, .. } = key.kind {
            *caller = AgentName::new("planner");
        }
        assert_ne!(base_delegate(), key);
    }

    #[test]
    fn delegate_differs_by_target() {
        let mut key = base_delegate();
        if let RoutingKind::Delegate { ref mut target, .. } = key.kind {
            *target = AgentName::new("fact-checker");
        }
        assert_ne!(base_delegate(), key);
    }

    // ========================================================================
    // Collision-freedom: DelegateReply (5 dimensions)
    // ========================================================================

    fn base_delegate_reply() -> RoutingKey {
        RoutingKey {
            session: session(),
            branch: branch(),
            submission: submission(),
            kind: RoutingKind::DelegateReply {
                caller: agent(),
                target: agent_alt(),
                nonce: Nonce::new("abc123"),
            },
        }
    }

    #[test]
    fn delegate_reply_equal_when_all_dimensions_match() {
        assert_eq!(base_delegate_reply(), base_delegate_reply());
    }

    #[test]
    fn delegate_reply_differs_by_session() {
        let mut key = base_delegate_reply();
        key.session = session_alt();
        assert_ne!(base_delegate_reply(), key);
    }

    #[test]
    fn delegate_reply_differs_by_nonce() {
        let mut key = base_delegate_reply();
        if let RoutingKind::DelegateReply { ref mut nonce, .. } = key.kind {
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

    // ========================================================================
    // Reply pairing (invariant 2): request → reply is deterministic
    // ========================================================================

    #[test]
    fn invoke_reply_is_complete() {
        let reply = base_invoke().reply_key(None).unwrap();
        assert_eq!(reply, base_complete());
    }

    #[test]
    fn invoke_reply_ignores_nonce() {
        let with = base_invoke()
            .reply_key(Some(Nonce::new("ignored")))
            .unwrap();
        let without = base_invoke().reply_key(None).unwrap();
        assert_eq!(with, without);
    }

    #[test]
    fn invoke_reply_drops_runtime() {
        // Complete doesn't carry RuntimeType — it goes back to harness, not runtime
        let reply = base_invoke().reply_key(None).unwrap();
        match reply.kind {
            RoutingKind::Complete { .. } => {}
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn request_reply_is_response() {
        let reply = base_request().reply_key(None).unwrap();
        assert_eq!(reply, base_response());
    }

    #[test]
    fn request_reply_preserves_all_dimensions() {
        let reply = base_request().reply_key(None).unwrap();
        assert_eq!(reply.session, self::session());
        assert_eq!(reply.branch, self::branch());
        assert_eq!(reply.submission, self::submission());
        match reply.kind {
            RoutingKind::Response {
                service,
                agent,
                operation,
                sequence,
            } => {
                assert_eq!(service, ServiceBackend::Kv(ObjectStorageType::Sqlite));
                assert_eq!(agent, self::agent());
                assert_eq!(operation, Operation::Get);
                assert_eq!(sequence, Sequence::first());
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn delegate_reply_is_delegate_reply_with_nonce() {
        let nonce = Nonce::new("abc123");
        let reply = base_delegate().reply_key(Some(nonce)).unwrap();
        assert_eq!(reply, base_delegate_reply());
    }

    #[test]
    fn delegate_without_nonce_returns_none() {
        assert!(base_delegate().reply_key(None).is_none());
    }

    #[test]
    fn reply_key_is_deterministic() {
        // Same inputs → same reply key, always
        let a = base_request().reply_key(None).unwrap();
        let b = base_request().reply_key(None).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_nonces_produce_different_delegate_replies() {
        let reply_a = base_delegate()
            .reply_key(Some(Nonce::new("nonce-1")))
            .unwrap();
        let reply_b = base_delegate()
            .reply_key(Some(Nonce::new("nonce-2")))
            .unwrap();
        assert_ne!(reply_a, reply_b);
    }

    // ========================================================================
    // Reply asymmetry (invariant 3): reply variants have no replies
    // ========================================================================

    #[test]
    fn complete_has_no_reply() {
        assert!(base_complete().reply_key(None).is_none());
    }

    #[test]
    fn response_has_no_reply() {
        assert!(base_response().reply_key(None).is_none());
    }

    #[test]
    fn delegate_reply_has_no_reply() {
        assert!(base_delegate_reply().reply_key(None).is_none());
    }

    #[test]
    fn delegate_reply_has_no_reply_even_with_nonce() {
        assert!(base_delegate_reply()
            .reply_key(Some(Nonce::new("wont-help")))
            .is_none());
    }
}
