//! CliHarness — CLI-specific implementation of the Harness trait (ADR 076).
//!
//! Domain types (Harness trait) live in `crate::domain`.
//! This module contains the CLI implementation with tick loop,
//! session management, and local filesystem support.

use std::collections::HashMap;
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
/// - `tick()`: Polling loop for reconciling completed jobs
/// - `start_session()`: Begin a conversation session (ADR 054)
/// - `record_response()`: Record agent response to in-memory session
pub struct CliHarness {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    inflight: HashMap<SubmissionId, JobId>,
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

impl CliHarness {
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
    ) -> Self {
        Self {
            queue,
            registry,
            inflight: HashMap::new(),
            session: None,
            last_submission_id: None,
            last_state: None,
            pending_state: None,
            timeline: TimelineId::main(),
            timeline_sealed: false,
        }
    }

    /// Start a conversation session for an agent (ADR 054, ADR 070).
    ///
    /// Creates a SessionId and Session. No ConversationStore — the harness
    /// computes SubmissionIds in-process and the GitDagWorker writes to git.
    pub fn start_session(&mut self, agent_name: &str) {
        let session_id = SessionId::new();
        let session = Session::new(session_id, agent_name);

        self.session = Some(session);
    }

    /// Record an agent response to the in-memory session.
    ///
    /// Clears the pending question and appends the completed turn to history.
    /// If state tracking is active (ADR 055), promotes pending_state to last_state.
    pub fn record_response(&mut self, response: &str) {
        if let Some(session) = self.session.as_mut() {
            // Take pending_state and promote it to last_state
            let state = self.pending_state.take();

            session.record_agent_response(response);

            if state.is_some() {
                self.last_state = state;
            }
        }
    }

    /// Set the initial state for the next invocation (ADR 055).
    ///
    /// Used by `--from` to fork from a historical state.
    pub fn set_initial_state(&mut self, state: String) {
        self.last_state = Some(state);
    }

    /// Set the timeline for branch-scoped subjects (ADR 093).
    ///
    /// If `sealed` is true, subsequent `invoke()` calls will be rejected.
    pub fn set_timeline(&mut self, timeline: TimelineId, sealed: bool) {
        self.timeline = timeline;
        self.timeline_sealed = sealed;
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
    /// Shared by `invoke()` (non-blocking) and `run_agent()` (blocking).
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
            HarnessType::Cli,
            runtime,
            AgentId::new(crate::domain::agent_routing_key(agent_id)),
            payload.as_bytes().to_vec(),
            self.last_state.clone(),
            invoke_diag,
        );

        Ok((invoke, job_id))
    }

    /// Run an agent to completion (ADR 092).
    ///
    /// Builds the invocation, sends it, and blocks until the CompleteMessage
    /// arrives. Returns the agent's output as a string.
    pub fn run_agent(&mut self, agent_id: &ResourceId, input: &str) -> Result<String, String> {
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

    /// Tick: monitor reply queue and update completed jobs in registry.
    ///
    /// CLI-specific: runs until no more messages or shutdown signal.
    /// Uses typed CompleteMessage (ADR 044) for job completion tracking.
    pub fn tick(&mut self) {
        // Poll each inflight submission's scoped consumer (ADR 052)
        let submissions: Vec<SubmissionId> = self.inflight.keys().cloned().collect();

        for submission in &submissions {
            while let Ok((complete, ack)) = self.queue.receive_complete(submission, "cli") {
                if let Some(job_id) = self.inflight.remove(&complete.submission) {
                    let result = String::from_utf8_lossy(&complete.payload).to_string();
                    self.registry.update_job_status(&job_id, JobStatus::Completed(result));
                    // Stash state from the completed invocation (ADR 055)
                    if complete.state.is_some() {
                        self.pending_state = complete.state;
                    }
                }
                let _ = ack();
            }
        }
    }
}

/// Read the latest state for an agent from the DAG store (ADR 079).
///
/// Queries the given DagStore for the most recent non-empty state hash
/// associated with the agent. Returns None if no state has been recorded.
pub fn read_latest_state(store: &dyn crate::domain::DagStore, agent_name: &str) -> Option<String> {
    store.latest_state(agent_name).ok().flatten()
}

impl Harness for CliHarness {
    fn harness_type(&self) -> HarnessType {
        HarnessType::Cli
    }

    fn deploy(&self, manifest_toml: &str) -> Result<ResourceId, String> {
        let manifest: AgentManifest = toml::from_str(manifest_toml)
            .map_err(|e| format!("failed to parse manifest: {}", e))?;

        let agent = self.registry.register_manifest(manifest)
            .map_err(|e| format!("registration failed: {}", e))?;

        Ok(agent.id)
    }

    fn invoke(&mut self, agent_id: &ResourceId, input: &str) -> Result<JobId, String> {
        let (invoke_msg, job_id) = self.build_invoke(agent_id, input)?;
        let submission = invoke_msg.submission.clone();

        self.queue
            .send_invoke(invoke_msg)
            .map_err(|e| format!("failed to queue: {}", e))?;

        self.inflight.insert(submission, job_id.clone());
        self.registry.update_job_status(&job_id, JobStatus::Running);

        Ok(job_id)
    }

    fn poll(&self, job_id: &JobId) -> Option<String> {
        match self.registry.get_job(job_id)?.status {
            JobStatus::Completed(ref result) => Some(result.clone()),
            JobStatus::Failed(ref error) => Some(format!("[error] {}", error)),
            _ => None,
        }
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

    fn test_agent_id() -> ResourceId {
        ResourceId::new("http://127.0.0.1:9000/agents/test-agent")
    }

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

    /// Deploy a minimal test agent.
    fn deploy_test_agent(harness: &CliHarness) -> ResourceId {
        let manifest = r#"
            name = "test-agent"
            description = "Test"
            runtime = "container"
            executable = "localhost/test-agent:latest"
            [requirements]

        "#;
        harness.deploy(manifest).unwrap()
    }

    #[test]
    fn harness_type_is_cli() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CliHarness::new(queue, registry);

        assert_eq!(harness.harness_type(), HarnessType::Cli);
    }

    #[test]
    fn invoke_creates_job_and_queues_message() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue.clone(), registry.clone());

        // Deploy agent first
        let agent_id = deploy_test_agent(&harness);

        // Invoke
        let job_id = harness.invoke(&agent_id, "hello").unwrap();

        // Job exists in registry with Running status
        let job = registry.get_job(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.agent_id, agent_id);

        // Message is receivable via trait method (ADR 096: no subject inspection)
        let (received, ack) = queue.receive_invoke("test-agent").unwrap();
        assert_eq!(received.harness, HarnessType::Cli);
        assert_eq!(received.runtime, RuntimeType::Container);
        assert_eq!(received.agent_id.as_str(), "test-agent");
        ack().unwrap();
    }

    #[test]
    fn invoke_fails_for_undeployed_agent() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue, registry);

        let agent_id = test_agent_id();
        let result = harness.invoke(&agent_id, "hello");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not deployed"));
    }

    #[test]
    fn poll_returns_none_for_running_job() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue, registry);

        let agent_id = deploy_test_agent(&harness);
        let job_id = harness.invoke(&agent_id, "hello").unwrap();

        // Poll returns None while job is running
        assert!(harness.poll(&job_id).is_none());
    }

    #[test]
    fn poll_returns_result_for_completed_job() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CliHarness::new(queue, registry.clone());

        let agent_id = test_agent_id();
        let job_id = registry.create_job(SubmissionId::new(), agent_id, "input".to_string());
        registry.update_job_status(&job_id, JobStatus::Completed("done".to_string()));

        assert_eq!(harness.poll(&job_id), Some("done".to_string()));
    }

    #[test]
    fn tick_reconciles_completed_jobs() {
        use crate::domain::ExpectsReply;

        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue.clone(), registry.clone());

        let agent_id = deploy_test_agent(&harness);
        let job_id = harness.invoke(&agent_id, "hello").unwrap();

        // Get the InvokeMessage via trait method to build CompleteMessage reply
        let (invoke, ack) = queue.receive_invoke("test-agent").unwrap();
        ack().unwrap();

        // Simulate runtime sending typed CompleteMessage (ADR 044)
        let complete = invoke.create_reply(b"result".to_vec());
        queue.send_complete(complete).unwrap();

        // Before tick: job is still Running
        assert_eq!(registry.get_job(&job_id).unwrap().status, JobStatus::Running);

        // Tick reconciles using typed receive
        harness.tick();

        // After tick: job is Completed
        assert_eq!(
            registry.get_job(&job_id).unwrap().status,
            JobStatus::Completed("result".to_string())
        );
    }

    #[test]
    fn deploy_from_path_loads_and_registers_agent() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CliHarness::new(queue, registry.clone());

        let agent_id = harness.deploy_from_path(&fixture_path("echo-agent")).unwrap();

        // Agent is registered
        let agent = registry.get_agent(&agent_id).unwrap();
        assert_eq!(agent.name, "echo-agent");
    }

    #[test]
    fn deploy_from_path_fails_for_nonexistent_path() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CliHarness::new(queue, registry);

        let result = harness.deploy_from_path(Path::new("/nonexistent/path"));

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to load agent"));
    }

    #[test]
    fn deploy_from_path_fails_for_path_without_manifest() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = CliHarness::new(queue, registry);

        // Use tests/ directory which exists but has no agent.toml
        let result = harness.deploy_from_path(Path::new("tests"));

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to load agent"));
    }

    // --- Session integration tests (ADR 070) ---

    #[test]
    fn invoke_with_session_creates_sha_derived_submission() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue.clone(), registry.clone());

        let agent_id = deploy_test_agent(&harness);

        harness.start_session("test-agent");

        let job_id = harness.invoke(&agent_id, "hello").unwrap();

        // Submission should be a valid hex SHA-256 (not a UUID with sub- prefix)
        let job = registry.get_job(&job_id).unwrap();
        let submission_str = job.submission_id.as_str();
        assert_eq!(submission_str.len(), 64, "SHA-256 should be 64 chars: {}", submission_str);
        assert!(submission_str.chars().all(|c| c.is_ascii_hexdigit()),
            "submission should be hex: {}", submission_str);
    }

    #[test]
    fn invoke_with_session_enriches_payload() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue.clone(), registry.clone());

        let agent_id = deploy_test_agent(&harness);

        harness.start_session("test-agent");

        harness.invoke(&agent_id, "first message").unwrap();

        // Check the payload via trait method
        let (invoke, ack) = queue.receive_invoke("test-agent").unwrap();
        ack().unwrap();
        let payload_str = String::from_utf8_lossy(&invoke.payload);

        // First message should just be "User: first message"
        assert_eq!(payload_str, "User: first message");
    }

    #[test]
    fn invoke_with_session_includes_history_on_second_turn() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue.clone(), registry.clone());

        let agent_id = deploy_test_agent(&harness);

        harness.start_session("test-agent");

        // First turn
        harness.invoke(&agent_id, "hello").unwrap();
        // Drain the first invoke so the next receive gets the second turn
        let (_first, ack) = queue.receive_invoke("test-agent").unwrap();
        ack().unwrap();
        harness.record_response("hi there");

        // Second turn — payload should include history
        harness.invoke(&agent_id, "follow up").unwrap();

        let (invoke, ack) = queue.receive_invoke("test-agent").unwrap();
        ack().unwrap();
        let payload_str = String::from_utf8_lossy(&invoke.payload);

        assert!(payload_str.contains("User: hello"), "payload: {}", payload_str);
        assert!(payload_str.contains("Agent: hi there"), "payload: {}", payload_str);
        assert!(payload_str.contains("User: follow up"), "payload: {}", payload_str);
    }

    #[test]
    fn second_invoke_has_different_submission_id() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue.clone(), registry.clone());

        let agent_id = deploy_test_agent(&harness);
        harness.start_session("test-agent");

        let job1 = harness.invoke(&agent_id, "first").unwrap();
        harness.record_response("reply");

        let job2 = harness.invoke(&agent_id, "second").unwrap();

        let sub1 = registry.get_job(&job1).unwrap().submission_id;
        let sub2 = registry.get_job(&job2).unwrap().submission_id;
        assert_ne!(sub1, sub2, "second invoke should have different submission");
    }

    // --- Timeline enforcement tests (ADR 093) ---

    #[test]
    fn sealed_timeline_rejects_invocation() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue, registry);

        let agent_id = deploy_test_agent(&harness);

        // Seal the timeline
        harness.set_timeline(TimelineId::from(42), true);

        let result = harness.invoke(&agent_id, "hello");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("sealed"));
    }

    #[test]
    fn unsealed_timeline_allows_invocation() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue, registry);

        let agent_id = deploy_test_agent(&harness);

        // Set non-sealed timeline
        harness.set_timeline(TimelineId::from(2), false);

        let result = harness.invoke(&agent_id, "hello");
        assert!(result.is_ok());
    }
}
