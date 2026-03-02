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
    HarnessType, InvokeDiagnostics, InvokeMessage, JobId, JobStatus, MessageQueue, Registry,
    ResourceId, SessionId, SubmissionId, TimelineId,
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

    /// Run an agent to completion synchronously.
    ///
    /// Sends input to the agent and blocks until the response arrives.
    /// Returns the agent's output as a string.
    fn run_agent(&mut self, agent_id: &ResourceId, input: &str) -> Result<String, String>;
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
}

impl CoreHarness {
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        harness_type: HarnessType,
    ) -> Self {
        Self {
            harness_type,
            queue,
            registry,
            session: None,
            last_submission_id: None,
            last_state: None,
            pending_state: None,
            timeline: TimelineId::main(),
            timeline_sealed: false,
        }
    }

    /// Record an agent response to the in-memory session.
    ///
    /// Clears the pending question and appends the completed turn to history.
    /// If state tracking is active (ADR 055), promotes pending_state to last_state.
    fn record_response(&mut self, response: &str) {
        if let Some(session) = self.session.as_mut() {
            // Take pending_state and promote it to last_state
            let state = self.pending_state.take();

            session.record_agent_response(response);

            if state.is_some() {
                self.last_state = state;
            }
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
            let enriched_payload = session.build_payload(input);
            let parent = self.last_submission_id.as_deref().unwrap_or("");
            let submission = SubmissionId::content_addressed(
                enriched_payload.as_bytes(),
                session.session.as_str(),
                parent,
            );
            session.record_user_input(input, submission.clone());
            self.last_submission_id = Some(submission.as_str().to_string());
            (submission, session.session.clone(), enriched_payload)
        } else {
            (SubmissionId::new(), SessionId::new(), input.to_string())
        };

        let job_id =
            self.registry
                .create_job(submission.clone(), agent_id.clone(), input.to_string());

        let history_turns = self
            .session
            .as_ref()
            .map(|s| s.history.len() as u32)
            .unwrap_or(0);
        let invoke_diag = InvokeDiagnostics {
            harness_version: env!("CARGO_PKG_VERSION").to_string(),
            history_turns,
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
        let session = Session::new(session_id, agent_name);

        self.session = Some(session);
    }

    fn set_initial_state(&mut self, state: String) {
        self.last_state = Some(state);
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
        self.record_response(&result);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{InMemoryRegistry, InMemorySecretStore, RuntimeType, SecretStore};
    use crate::queue::InMemoryQueue;

    #[test]
    fn harness_type_is_cli() {
        let queue = Arc::new(InMemoryQueue::new());
        let store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let registry = InMemoryRegistry::new(store);
        registry.register_runtime(RuntimeType::Container);
        let registry: Arc<dyn Registry> = Arc::new(registry);

        let harness = CoreHarness::new(queue, registry, HarnessType::Cli);

        assert_eq!(harness.harness_type(), HarnessType::Cli);
    }
}
