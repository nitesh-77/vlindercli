//! Harness - API surface for the system.
//!
//! The harness is the entry point for external requests. Different harness
//! types handle different interfaces (CLI, Web API, WhatsApp, etc.) but share
//! a common contract via the `Harness` trait.
//!
//! - `Harness` trait: Common operations (deploy, invoke, poll)
//! - `CliHarness`: Command-line implementation with tick loop and local paths

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::domain::registry::{JobId, JobStatus, Registry};
use crate::domain::{Agent, ResourceId};
use crate::queue::{
    HarnessType, InvokeMessage, MessageQueue, SubmissionId,
};

/// Common harness operations shared across all harness types.
pub trait Harness {
    /// The type of this harness (CLI, Web, API, WhatsApp).
    fn harness_type(&self) -> HarnessType;

    /// Deploy an agent from TOML manifest content.
    fn deploy(&self, manifest_toml: &str) -> Result<ResourceId, String>;

    /// Submit a job for an already-deployed agent.
    fn invoke(&mut self, agent_id: &ResourceId, input: &str) -> Result<JobId, String>;

    /// Poll for job completion.
    fn poll(&self, job_id: &JobId) -> Option<String>;
}

/// CLI harness implementation.
///
/// Adds CLI-specific functionality:
/// - `deploy_from_path()`: Load agent from local filesystem
/// - `tick()`: Polling loop for reconciling completed jobs
pub struct CliHarness {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    inflight: HashMap<SubmissionId, JobId>,
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
        }
    }

    /// Deploy an agent from a local directory path (CLI-specific).
    ///
    /// Loads the manifest, resolves relative paths, validates requirements,
    /// and registers the agent.
    pub fn deploy_from_path(&self, path: &Path) -> Result<ResourceId, String> {
        let agent = Agent::load(path)
            .map_err(|e| format!("failed to load agent: {:?}", e))?;

        self.register_agent(agent)
    }

    /// Internal: register an agent after loading/parsing.
    ///
    /// Idempotent: if an agent with the same name is already registered and
    /// the configuration matches, returns the existing ID. If the configuration
    /// differs, returns an error listing the differences.
    fn register_agent(&self, agent: Agent) -> Result<ResourceId, String> {
        let name = agent.name.clone();

        if let Some(existing) = self.registry.get_agent_by_name(&name) {
            let diffs = compare_agents(&agent, &existing);
            if diffs.is_empty() {
                return Ok(existing.id);
            }
            return Err(format!(
                "agent '{}' is already deployed with a different configuration:\n{}",
                name,
                diffs.join("\n")
            ));
        }

        self.registry.register_agent(agent)
            .map_err(|e| format!("registration failed: {}", e))?;

        // Query after registration — in distributed mode, the server assigns the ID.
        Ok(self.registry.agent_id(&name))
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
                }
                let _ = ack();
            }
        }
    }
}

/// Compare two agents and return a list of field differences.
///
/// Skips `id` (placeholder vs registry-assigned) and `mounts` (path-dependent).
/// Returns an empty vec if the agents are functionally identical.
fn compare_agents(new: &Agent, existing: &Agent) -> Vec<String> {
    let mut diffs = Vec::new();

    if new.executable != existing.executable {
        diffs.push(format!("  - executable: {:?} -> {:?}", existing.executable, new.executable));
    }
    if new.runtime != existing.runtime {
        diffs.push(format!("  - runtime: {:?} -> {:?}", existing.runtime, new.runtime));
    }
    if new.description != existing.description {
        diffs.push(format!("  - description: {:?} -> {:?}", existing.description, new.description));
    }
    if new.object_storage != existing.object_storage {
        diffs.push(format!("  - object_storage: {:?} -> {:?}", existing.object_storage, new.object_storage));
    }
    if new.vector_storage != existing.vector_storage {
        diffs.push(format!("  - vector_storage: {:?} -> {:?}", existing.vector_storage, new.vector_storage));
    }
    if new.requirements.models != existing.requirements.models {
        diffs.push(format!("  - requirements.models: {:?} -> {:?}", existing.requirements.models, new.requirements.models));
    }
    if new.requirements.services != existing.requirements.services {
        diffs.push(format!("  - requirements.services: {:?} -> {:?}", existing.requirements.services, new.requirements.services));
    }

    diffs
}

impl Harness for CliHarness {
    fn harness_type(&self) -> HarnessType {
        HarnessType::Cli
    }

    fn deploy(&self, manifest_toml: &str) -> Result<ResourceId, String> {
        let agent = Agent::from_toml(manifest_toml)
            .map_err(|e| format!("failed to parse manifest: {:?}", e))?;

        self.register_agent(agent)
    }

    fn invoke(&mut self, agent_id: &ResourceId, input: &str) -> Result<JobId, String> {
        // Verify agent is deployed and get runtime type
        let agent = self.registry.get_agent(agent_id)
            .ok_or_else(|| format!("agent not deployed: {}", agent_id))?;
        let runtime = self.registry.select_runtime(&agent)
            .ok_or_else(|| format!("no runtime available for agent: {}", agent_id))?;

        // Create submission context (ADR 044) - shared by job and message flow
        let submission = SubmissionId::new();

        // Create job in registry with submission tracking
        let job_id = self.registry.create_job(submission.clone(), agent_id.clone(), input.to_string());

        // Build and send typed InvokeMessage (ADR 044)
        let invoke = InvokeMessage::new(
            submission.clone(),
            HarnessType::Cli,
            runtime,
            agent_id.clone(),
            input.as_bytes().to_vec(),
        );

        self.queue
            .send_invoke(invoke)
            .map_err(|e| format!("failed to queue: {}", e))?;

        // Track submission → job for completion reconciliation (ADR 052)
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
    use crate::domain::{InMemoryRegistry, RuntimeType};
    use crate::queue::InMemoryQueue;
    use std::path::PathBuf;

    fn test_agent_id() -> ResourceId {
        ResourceId::new("http://127.0.0.1:9000/agents/test-agent")
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from("tests/fixtures/agents").join(name)
    }

    /// Create a registry with Container runtime registered (required for agent deployment).
    fn test_registry() -> Arc<dyn Registry> {
        let registry = InMemoryRegistry::new();
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
            services = []
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

        // Message is in typed queue with ADR 044 subject pattern
        // Agent name extracted from registry ID "...agents/test-agent" → "test-agent"
        let typed = queue.typed_queues.lock().unwrap();
        assert_eq!(typed.len(), 1);
        let (subject, _) = typed.iter().next().unwrap();
        assert!(subject.contains(".invoke."));
        assert!(subject.contains(".cli."));
        assert!(subject.contains(".container."));
        assert!(subject.contains(".test-agent"), "subject should contain agent name: {}", subject);
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
        use crate::queue::ExpectsReply;

        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = CliHarness::new(queue.clone(), registry.clone());

        let agent_id = deploy_test_agent(&harness);
        let job_id = harness.invoke(&agent_id, "hello").unwrap();

        // Get the InvokeMessage from typed queue to build CompleteMessage reply
        let invoke = {
            let typed = queue.typed_queues.lock().unwrap();
            let (_, messages) = typed.iter().next().unwrap();
            match &messages[0] {
                crate::queue::ObservableMessage::Invoke(msg) => msg.clone(),
                _ => panic!("expected InvokeMessage"),
            }
        };

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
}
