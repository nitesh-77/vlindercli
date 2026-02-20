//! CoreHarness — CLI-specific implementation of the Harness trait (ADR 076).
//!
//! Domain types (Harness trait) live in `crate::domain`.
//! This module contains the CLI implementation with session management
//! and local filesystem support.

use std::path::Path;
use std::sync::Arc;

use crate::domain::{
    AgentId, AgentManifest, Harness, HarnessType, InvokeDiagnostics, InvokeMessage,
    JobId, JobStatus, MessageQueue, Registry, ResourceId,
    SessionId, SubmissionId, TimelineId,
};
use crate::domain::Session;

/// CLI harness implementation.
///
/// Adds CLI-specific functionality:
/// - `deploy_from_path()`: Load agent from local filesystem
/// - `start_session()`: Begin a conversation session (ADR 054)
/// - `record_response()`: Record agent response to in-memory session
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

    /// Deploy an agent from a local directory path (CLI-specific).
    ///
    /// Loads the manifest, validates requirements, and registers via the
    /// registry's `register_manifest()` (ADR 102). Idempotency is handled
    /// by the registry: same manifest → returns existing agent.
    pub fn deploy_from_path(&self, path: &Path) -> Result<ResourceId, String> {
        let manifest_path = path.join("agent.toml");
        let manifest = AgentManifest::load(&manifest_path)
            .map_err(|e| format!("failed to load agent: {:?}", e))?;

        let agent = self.registry.register_manifest(manifest)
            .map_err(|e| format!("registration failed: {}", e))?;

        Ok(agent.id)
    }

    /// Build an InvokeMessage from session state and register a job.
    ///
    /// Returns the message and the job ID.
    fn build_invoke(&mut self, agent_id: &ResourceId, input: &str) -> Result<(InvokeMessage, JobId), String> {
        // Reject invocations on sealed timelines (ADR 093)
        if self.timeline_sealed {
            return Err(
                "Timeline is sealed. Use `vlinder timeline repair` to fork a new timeline.".to_string()
            );
        }

        let agent = self.registry.get_agent(agent_id)
            .ok_or_else(|| format!("agent not deployed: {}", agent_id))?;
        let runtime = self.registry.select_runtime(&agent)
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

        let job_id = self.registry.create_job(submission.clone(), agent_id.clone(), input.to_string());

        let history_turns = self.session.as_ref()
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
            AgentId::new(crate::domain::agent_routing_key(agent_id)),
            payload.as_bytes().to_vec(),
            self.last_state.clone(),
            invoke_diag,
        );

        Ok((invoke, job_id))
    }
}

/// Read the latest state for an agent from the DAG store (ADR 079).
///
/// Queries the given DagStore for the most recent non-empty state hash
/// associated with the agent. Returns None if no state has been recorded.
pub fn read_latest_state(store: &dyn crate::domain::DagStore, agent_name: &str) -> Option<String> {
    store.latest_state(agent_name).ok().flatten()
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

        let complete = self.queue.run_agent(invoke_msg)
            .map_err(|e| format!("queue error: {}", e))?;

        let result = String::from_utf8_lossy(&complete.payload).to_string();
        self.registry.update_job_status(&job_id, JobStatus::Completed(result.clone()));
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
    use crate::registry::InMemoryRegistry;
    use crate::queue::InMemoryQueue;
    use crate::secret_store::InMemorySecretStore;
    use crate::domain::{RuntimeType, SecretStore};
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from("tests/fixtures/agents").join(name)
    }

    /// Create a registry with Container runtime registered (required for agent deployment).
    fn test_registry() -> Arc<dyn Registry> {
        let store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let registry = InMemoryRegistry::new(store);
        registry.register_runtime(RuntimeType::Container);
        Arc::new(registry)
    }

    #[test]
    fn harness_type_is_cli() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CoreHarness::new(queue, registry, HarnessType::Cli);

        assert_eq!(harness.harness_type(), HarnessType::Cli);
    }

    #[test]
    fn deploy_from_path_loads_and_registers_agent() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CoreHarness::new(queue, registry.clone(), HarnessType::Cli);

        let agent_id = harness.deploy_from_path(&fixture_path("echo-agent")).unwrap();

        // Agent is registered
        let agent = registry.get_agent(&agent_id).unwrap();
        assert_eq!(agent.name, "echo-agent");
    }

    #[test]
    fn deploy_from_path_fails_for_nonexistent_path() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CoreHarness::new(queue, registry, HarnessType::Cli);

        let result = harness.deploy_from_path(Path::new("/nonexistent/path"));

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to load agent"));
    }

    #[test]
    fn deploy_from_path_fails_for_path_without_manifest() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CoreHarness::new(queue, registry, HarnessType::Cli);

        // Use tests/ directory which exists but has no agent.toml
        let result = harness.deploy_from_path(Path::new("tests"));

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to load agent"));
    }
}
