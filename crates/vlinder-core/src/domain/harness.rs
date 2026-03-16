//! Harness - API surface for agent interaction.
//!
//! The harness is the entry point for external requests. Different harness
//! types handle different interfaces (CLI, Web API, WhatsApp, etc.) but share
//! a common contract via the `Harness` trait.
//!
//! `CoreHarness` is the canonical implementation: it orchestrates sessions,
//! Merkle-chained submissions, job lifecycle, and timeline sealing.

use std::sync::Arc;

use crate::domain::{
    BranchId, DagNodeId, DagStore, ForkMessage, HarnessType, InvokeDiagnostics, InvokeMessage,
    JobId, JobStatus, MessageQueue, MessageType, Registry, ResourceId, SessionId,
    SessionStartMessage, SubmissionId,
};

/// Common harness operations shared across all harness types.
pub trait Harness {
    /// Identify which transport submitted the job.
    ///
    /// Stamped into every `InvokeMessage` and used by the completion
    /// path to route responses back to the correct consumer.
    fn harness_type(&self) -> HarnessType;

    /// Start a new conversation session for an agent.
    ///
    /// Creates a session and its default "main" branch. Returns the
    /// SessionId and the default branch's BranchId.
    fn start_session(&self, agent_name: &str) -> (SessionId, BranchId);

    /// Run an agent to completion synchronously.
    ///
    /// Sends input to the agent and blocks until the response arrives.
    /// Returns the agent's output as a string.
    #[allow(clippy::too_many_arguments)]
    fn run_agent(
        &self,
        agent_id: &ResourceId,
        input: &str,
        session_id: SessionId,
        timeline: BranchId,
        sealed: bool,
        initial_state: Option<String>,
        dag_parent: DagNodeId,
    ) -> Result<String, String>;

    /// Create a timeline fork by sending a ForkMessage through the queue.
    ///
    /// Fire-and-forget: both SQL (via RecordingQueue) and git (via
    /// GitDagWorker) react to the message. No response is expected.
    fn fork_timeline(
        &self,
        params: ForkParams,
        session_id: SessionId,
        timeline: BranchId,
    ) -> Result<(), String>;
}

/// Parameters for `Harness::fork_timeline()`.
///
/// The CLI reads these from the DagStore (node lookup + session context).
/// The harness wraps them in a ForkMessage and sends through the queue.
pub struct ForkParams {
    pub agent_name: String,
    pub branch_name: String,
    pub fork_point: DagNodeId,
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
        }
    }

    /// Build an InvokeMessage from session state and register a job.
    ///
    /// Returns the message and the job ID.
    #[allow(clippy::too_many_arguments)]
    fn build_invoke(
        &self,
        agent_id: &ResourceId,
        input: &str,
        session_id: &SessionId,
        timeline: &BranchId,
        sealed: bool,
        initial_state: Option<&str>,
        dag_parent: &DagNodeId,
    ) -> Result<(InvokeMessage, JobId), String> {
        // Reject invocations on sealed timelines (ADR 093)
        if sealed {
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

        let last_invoke_node = self
            .store
            .latest_node_on_branch(*timeline, Some(MessageType::Invoke))
            .unwrap_or(None);
        let last_invoke_payload = last_invoke_node
            .as_ref()
            .map(|n| String::from_utf8_lossy(n.payload()).to_string());
        let last_complete_node = self
            .store
            .latest_node_on_branch(*timeline, Some(MessageType::Complete))
            .unwrap_or(None);
        let last_complete_payload = last_complete_node
            .as_ref()
            .map(|n| String::from_utf8_lossy(n.payload()).to_string());
        let enriched_payload = build_payload(
            last_invoke_payload.as_deref(),
            last_complete_payload.as_deref(),
            input,
        );
        let parent_submission = last_invoke_node
            .as_ref()
            .map(|n| n.submission_id().as_str().to_string())
            .unwrap_or_default();
        let submission = SubmissionId::content_addressed(
            enriched_payload.as_bytes(),
            session_id.as_str(),
            &parent_submission,
        );
        // State: prefer DAG's latest Complete, fall back to initial_state
        let last_state = last_complete_node
            .as_ref()
            .and_then(|n| n.message.state().map(|s| s.to_string()))
            .or_else(|| initial_state.map(|s| s.to_string()));

        let job_id =
            self.registry
                .create_job(submission.clone(), agent_id.clone(), input.to_string());

        let invoke_diag = InvokeDiagnostics {
            harness_version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let invoke = InvokeMessage::new(
            *timeline,
            submission,
            session_id.clone(),
            self.harness_type(),
            runtime,
            crate::domain::agent_routing_key(agent_id),
            enriched_payload.as_bytes().to_vec(),
            last_state,
            invoke_diag,
            dag_parent.clone(),
        );

        Ok((invoke, job_id))
    }
}

impl Harness for CoreHarness {
    fn harness_type(&self) -> HarnessType {
        self.harness_type
    }

    fn start_session(&self, agent_name: &str) -> (SessionId, BranchId) {
        let session_id = SessionId::new();
        // Placeholder — the actual branch ID is determined by
        // send_session_start (RecordingQueue creates the branch row).
        let placeholder = BranchId::from(0);

        let msg = SessionStartMessage::new(placeholder, session_id.clone(), agent_name.to_string());
        let branch_id = self.queue.send_session_start(msg).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to send session start message");
            BranchId::from(1) // fallback
        });

        (session_id, branch_id)
    }

    fn run_agent(
        &self,
        agent_id: &ResourceId,
        input: &str,
        session_id: SessionId,
        timeline: BranchId,
        sealed: bool,
        initial_state: Option<String>,
        dag_parent: DagNodeId,
    ) -> Result<String, String> {
        let (invoke_msg, job_id) = self.build_invoke(
            agent_id,
            input,
            &session_id,
            &timeline,
            sealed,
            initial_state.as_deref(),
            &dag_parent,
        )?;
        self.registry.update_job_status(&job_id, JobStatus::Running);

        let complete = self
            .queue
            .run_agent(invoke_msg)
            .map_err(|e| format!("queue error: {}", e))?;

        let result = String::from_utf8_lossy(&complete.payload).to_string();
        self.registry
            .update_job_status(&job_id, JobStatus::Completed(result.clone()));
        Ok(result)
    }

    fn fork_timeline(
        &self,
        params: ForkParams,
        session_id: SessionId,
        timeline: BranchId,
    ) -> Result<(), String> {
        let submission = SubmissionId::new();

        let fork_msg = ForkMessage::new(
            timeline,
            submission,
            session_id,
            params.agent_name,
            params.branch_name,
            params.fork_point,
        );

        self.queue
            .send_fork(fork_msg)
            .map_err(|e| format!("queue error: {}", e))
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
