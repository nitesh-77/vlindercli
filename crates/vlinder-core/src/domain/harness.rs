//! Harness - API surface for agent interaction.
//!
//! The harness is the entry point for external requests. Different harness
//! types handle different interfaces (CLI, Web API, WhatsApp, etc.) but share
//! a common contract via the `Harness` trait.
//!
//! `CoreHarness` is the canonical implementation: it orchestrates sessions,
//! Merkle-chained submissions, job lifecycle, and timeline sealing.

use std::sync::Arc;

use crate::domain::Session;
use crate::domain::{
    AgentId, DagNodeId, DagStore, ForkMessage, HarnessType, InvokeDiagnostics, InvokeMessage,
    JobId, JobStatus, MessageQueue, MessageType, Operation, Registry, RepairMessage, ResourceId,
    Sequence, ServiceBackend, SessionId, SessionStartMessage, SubmissionId, TimelineId,
};

/// Common harness operations shared across all harness types.
pub trait Harness {
    /// Identify which transport submitted the job.
    ///
    /// Stamped into every `InvokeMessage` and used by the completion
    /// path to route responses back to the correct consumer.
    fn harness_type(&self) -> HarnessType;

    /// Set the timeline for branch-scoped subjects (ADR 093).
    ///
    /// If `sealed` is true, subsequent invocations will be rejected.
    fn set_timeline(&mut self, timeline: TimelineId, sealed: bool);

    /// Start a conversation session for an agent.
    ///
    /// Creates a session that tracks conversation history, submission
    /// chaining, and state continuity across turns.
    fn start_session(&mut self, agent_name: &str);

    /// Set the initial state for the next invocation.
    ///
    /// Used to resume from a historical state (time travel, session
    /// continuity). The state hash is passed to the agent on the next invoke.
    fn set_initial_state(&mut self, state: String);

    /// Set the DAG parent commit hash for the next invocation.
    ///
    /// The harness stamps this into every InvokeMessage. The GitDagWorker
    /// uses it as the parent for the invoke commit instead of its cached
    /// last_commit. For normal invokes this is the current DAG tip; for
    /// repair it will be the checked-out commit.
    fn set_dag_parent(&mut self, id: DagNodeId);

    /// Run an agent to completion synchronously.
    ///
    /// Sends input to the agent and blocks until the response arrives.
    /// Returns the agent's output as a string.
    fn run_agent(&mut self, agent_id: &ResourceId, input: &str) -> Result<String, String>;

    /// Replay a failed service call (ADR 113).
    ///
    /// The caller provides the repair metadata (read from the DAG at the
    /// checkout point). The harness adds its own context (timeline, session,
    /// harness type) and sends a RepairMessage through the queue.
    ///
    /// Returns the agent's output after the checkpoint handler processes
    /// the replayed service response.
    fn repair_agent(&mut self, params: RepairParams) -> Result<String, String>;

    /// Create a timeline fork by sending a ForkMessage through the queue.
    ///
    /// Fire-and-forget: both SQL (via RecordingQueue) and git (via
    /// GitDagWorker) react to the message. No response is expected.
    fn fork_timeline(&mut self, params: ForkParams) -> Result<(), String>;
}

/// Parameters for `Harness::repair_agent()`.
///
/// The caller (CLI) reads these from the DAG at the checkout point.
/// The harness adds timeline, session, submission, and harness type.
pub struct RepairParams {
    pub agent_id: AgentId,
    pub dag_parent: DagNodeId,
    pub checkpoint: String,
    pub service: ServiceBackend,
    pub operation: Operation,
    pub sequence: Sequence,
    pub payload: Vec<u8>,
    pub state: Option<String>,
}

/// Parameters for `Harness::fork_timeline()`.
///
/// The CLI reads these from the DagStore (node lookup + session context).
/// The harness wraps them in a ForkMessage and sends through the queue.
pub struct ForkParams {
    pub agent_name: String,
    pub branch_name: String,
    pub fork_point: DagNodeId,
    pub parent_timeline_id: i64,
}

/// Build an enriched payload from DAG-derived history.
///
/// Each invoke payload already contains the full conversation history up to
/// that point, so we only need the last invoke + last complete to reconstruct.
fn build_payload(
    last_invoke_payload: Option<&str>,
    last_complete_payload: Option<&str>,
    current_input: &str,
) -> String {
    match (last_invoke_payload, last_complete_payload) {
        (Some(invoke), Some(complete)) => {
            format!("{}\nAgent: {}\nUser: {}", invoke, complete, current_input)
        }
        _ => format!("User: {}", current_input),
    }
}

// ============================================================================
// CoreHarness — canonical implementation
// ============================================================================

/// Core harness implementation.
///
/// Orchestrates the full invocation lifecycle:
/// - Session management with conversation history
/// - Content-addressed submission chaining (ADR 081)
/// - State tracking with pending/committed promotion (ADR 055)
/// - Timeline-scoped invocations with seal enforcement (ADR 093)
/// - Job creation and status tracking via the registry
pub struct CoreHarness {
    harness_type: HarnessType,
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    store: Arc<dyn DagStore>,
    session: Option<Session>,
    /// Last SubmissionId — parent for Merkle chaining (ADR 081).
    last_submission_id: Option<String>,
    /// Final state hash from the last completed invocation (ADR 055).
    last_state: Option<String>,
    /// Pending state from a just-completed invocation, not yet committed.
    pending_state: Option<String>,
    /// Timeline ID for branch-scoped subjects (ADR 093).
    timeline: TimelineId,
    /// Whether the current timeline is sealed (ADR 093).
    /// Sealed timelines reject new invocations.
    timeline_sealed: bool,
    /// DagNode to parent the next invoke on.
    dag_parent: DagNodeId,
}

impl CoreHarness {
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        store: Arc<dyn DagStore>,
        harness_type: HarnessType,
    ) -> Self {
        Self {
            harness_type,
            queue,
            registry,
            store,
            session: None,
            last_submission_id: None,
            last_state: None,
            pending_state: None,
            timeline: TimelineId::main(),
            timeline_sealed: false,
            dag_parent: DagNodeId::root(),
        }
    }

    /// Promote pending state after a completed invocation (ADR 055).
    fn record_response(&mut self) {
        let state = self.pending_state.take();
        if state.is_some() {
            self.last_state = state;
        }
    }

    /// Build an InvokeMessage from session state and register a job.
    ///
    /// Returns the message and the job ID.
    fn build_invoke(
        &mut self,
        agent_id: &ResourceId,
        input: &str,
    ) -> Result<(InvokeMessage, JobId), String> {
        // Reject invocations on sealed timelines (ADR 093)
        if self.timeline_sealed {
            return Err(
                "Timeline is sealed. Use `vlinder timeline repair` to fork a new timeline."
                    .to_string(),
            );
        }

        let agent = self
            .registry
            .get_agent(agent_id)
            .ok_or_else(|| format!("agent not deployed: {}", agent_id))?;
        let runtime = self
            .registry
            .select_runtime(&agent)
            .ok_or_else(|| format!("no runtime available for agent: {}", agent_id))?;

        let (submission, session_id, payload) = if let Some(session) = self.session.as_mut() {
            let timeline_id: i64 = self.timeline.as_str().parse().unwrap_or(0);
            let last_invoke_payload = self
                .store
                .latest_node_on_timeline(timeline_id, Some(MessageType::Invoke))
                .unwrap_or(None)
                .map(|n| String::from_utf8_lossy(n.payload()).to_string());
            let last_complete_payload = self
                .store
                .latest_node_on_timeline(timeline_id, Some(MessageType::Complete))
                .unwrap_or(None)
                .map(|n| String::from_utf8_lossy(n.payload()).to_string());
            let enriched_payload = build_payload(
                last_invoke_payload.as_deref(),
                last_complete_payload.as_deref(),
                input,
            );
            let parent = self.last_submission_id.as_deref().unwrap_or("");
            let submission = SubmissionId::content_addressed(
                enriched_payload.as_bytes(),
                session.session.as_str(),
                parent,
            );
            self.last_submission_id = Some(submission.as_str().to_string());
            (submission, session.session.clone(), enriched_payload)
        } else {
            (SubmissionId::new(), SessionId::new(), input.to_string())
        };

        let job_id =
            self.registry
                .create_job(submission.clone(), agent_id.clone(), input.to_string());

        let invoke_diag = InvokeDiagnostics {
            harness_version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let invoke = InvokeMessage::new(
            self.timeline.clone(),
            submission,
            session_id,
            self.harness_type(),
            runtime,
            crate::domain::agent_routing_key(agent_id),
            payload.as_bytes().to_vec(),
            self.last_state.clone(),
            invoke_diag,
            self.dag_parent.clone(),
        );

        Ok((invoke, job_id))
    }
}

impl Harness for CoreHarness {
    fn harness_type(&self) -> HarnessType {
        self.harness_type
    }

    fn set_timeline(&mut self, timeline: TimelineId, sealed: bool) {
        self.timeline = timeline;
        self.timeline_sealed = sealed;
    }

    fn start_session(&mut self, agent_name: &str) {
        let session_id = SessionId::new();
        let session = Session::new(session_id.clone(), agent_name);

        let msg =
            SessionStartMessage::new(self.timeline.clone(), session_id, agent_name.to_string());
        if let Err(e) = self.queue.send_session_start(msg) {
            tracing::warn!(error = %e, "Failed to send session start message");
        }

        self.session = Some(session);
    }

    fn set_initial_state(&mut self, state: String) {
        self.last_state = Some(state);
    }

    fn set_dag_parent(&mut self, id: DagNodeId) {
        self.dag_parent = id;
    }

    fn run_agent(&mut self, agent_id: &ResourceId, input: &str) -> Result<String, String> {
        let (invoke_msg, job_id) = self.build_invoke(agent_id, input)?;
        self.registry.update_job_status(&job_id, JobStatus::Running);

        let complete = self
            .queue
            .run_agent(invoke_msg)
            .map_err(|e| format!("queue error: {}", e))?;

        let result = String::from_utf8_lossy(&complete.payload).to_string();
        self.registry
            .update_job_status(&job_id, JobStatus::Completed(result.clone()));
        if complete.state.is_some() {
            self.pending_state = complete.state;
        }
        self.record_response();
        Ok(result)
    }

    fn fork_timeline(&mut self, params: ForkParams) -> Result<(), String> {
        let session_id = self
            .session
            .as_ref()
            .map(|s| s.session.clone())
            .unwrap_or_default();

        let submission = SubmissionId::new();

        let fork_msg = ForkMessage::new(
            self.timeline.clone(),
            submission,
            session_id,
            params.agent_name,
            params.branch_name,
            params.fork_point,
            params.parent_timeline_id,
        );

        self.queue
            .send_fork(fork_msg)
            .map_err(|e| format!("queue error: {}", e))
    }

    fn repair_agent(&mut self, params: RepairParams) -> Result<String, String> {
        let session_id = self
            .session
            .as_ref()
            .map(|s| s.session.clone())
            .unwrap_or_default();

        let submission = SubmissionId::content_addressed(
            &params.payload,
            session_id.as_str(),
            self.last_submission_id.as_deref().unwrap_or(""),
        );

        let repair_msg = RepairMessage::new(
            self.timeline.clone(),
            submission,
            session_id,
            params.agent_id,
            self.harness_type(),
            params.dag_parent,
            params.checkpoint,
            params.service,
            params.operation,
            params.sequence,
            params.payload,
            params.state,
        );

        let complete = self
            .queue
            .repair_agent(repair_msg)
            .map_err(|e| format!("queue error: {}", e))?;

        let result = String::from_utf8_lossy(&complete.payload).to_string();
        if complete.state.is_some() {
            self.pending_state = complete.state;
        }
        self.record_response();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        InMemoryDagStore, InMemoryRegistry, InMemorySecretStore, RuntimeType, SecretStore,
    };
    use crate::queue::InMemoryQueue;

    #[test]
    fn harness_type_is_cli() {
        let queue = Arc::new(InMemoryQueue::new());
        let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let registry = InMemoryRegistry::new(secret_store);
        registry.register_runtime(RuntimeType::Container);
        let registry: Arc<dyn Registry> = Arc::new(registry);
        let store: Arc<dyn DagStore> = Arc::new(InMemoryDagStore::new());

        let harness = CoreHarness::new(queue, registry, store, HarnessType::Cli);

        assert_eq!(harness.harness_type(), HarnessType::Cli);
    }
}
