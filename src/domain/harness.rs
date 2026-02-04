//! Harness - API surface for the system.
//!
//! The harness is the single entry point for external requests:
//! - deploy: Register an agent after validating requirements
//! - invoke: Submit a job for execution
//! - poll: Check job status
//! - tick: Reconcile completed jobs from reply queue

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::domain::registry::{JobId, JobStatus, Registry};
use crate::domain::{Agent, ResourceId};
use crate::queue::{Message, MessageId, MessageQueue};

/// The harness - API surface for agent deployment and job management.
pub struct Harness {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    reply_queue: String,
    inflight: HashMap<MessageId, JobId>,
}

impl Harness {
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
    ) -> Self {
        Self {
            queue,
            registry,
            reply_queue: format!("harness-{}", uuid::Uuid::new_v4()),
            inflight: HashMap::new(),
        }
    }

    /// Deploy an agent from a local directory path.
    ///
    /// Loads the manifest, resolves relative paths, validates requirements,
    /// and registers the agent. This is the preferred method for CLI usage.
    pub fn deploy_from_path(&self, path: &Path) -> Result<ResourceId, String> {
        let agent = Agent::load(path)
            .map_err(|e| format!("failed to load agent: {:?}", e))?;

        self.register_agent(agent)
    }

    /// Deploy an agent from TOML content with pre-resolved URIs.
    ///
    /// Use this when the manifest comes from a non-filesystem source (e.g., HTTP request)
    /// where URIs are already absolute.
    pub fn deploy(&self, manifest_toml: &str) -> Result<ResourceId, String> {
        let agent = Agent::from_toml(manifest_toml)
            .map_err(|e| format!("failed to parse manifest: {:?}", e))?;

        self.register_agent(agent)
    }

    /// Internal: register an agent after loading/parsing.
    fn register_agent(&self, agent: Agent) -> Result<ResourceId, String> {
        let agent_id = agent.id.clone();

        self.registry.register_agent(agent)
            .map_err(|e| format!("registration failed: {}", e))?;

        Ok(agent_id)
    }

    /// Submit a job for an already-deployed agent.
    ///
    /// Creates job in registry, queues message, returns job ID.
    pub fn invoke(&mut self, agent_id: &ResourceId, input: &str) -> Result<JobId, String> {
        // Verify agent is deployed
        if self.registry.get_agent(agent_id).is_none() {
            return Err(format!("agent not deployed: {}", agent_id));
        }

        // Create job in registry
        let job_id = self.registry.create_job(agent_id.clone(), input.to_string());

        // Queue message to agent
        let message = Message::request(input.as_bytes().to_vec(), &self.reply_queue);
        let message_id = message.id.clone();

        tracing::info!(
            agent_id = %agent_id,
            reply_queue = %self.reply_queue,
            "Sending message to agent queue"
        );

        self.queue
            .send(agent_id.as_str(), message)
            .map_err(|e| format!("failed to queue: {}", e))?;

        tracing::info!(agent_id = %agent_id, "Message sent successfully");

        // Track for response correlation
        self.inflight.insert(message_id, job_id.clone());
        self.registry.update_job_status(&job_id, JobStatus::Running);

        Ok(job_id)
    }

    /// Poll for job completion.
    pub fn poll(&self, job_id: &JobId) -> Option<String> {
        match self.registry.get_job(job_id)?.status {
            JobStatus::Completed(ref result) => Some(result.clone()),
            JobStatus::Failed(ref error) => Some(format!("[error] {}", error)),
            _ => None,
        }
    }

    /// Tick: monitor reply queue and update completed jobs in registry.
    pub fn tick(&mut self) {
        tracing::debug!(reply_queue = %self.reply_queue, "Harness tick: checking for responses");
        while let Ok(pending) = self.queue.receive(&self.reply_queue) {
            if let Some(correlation_id) = &pending.message.correlation_id {
                if let Some(job_id) = self.inflight.remove(correlation_id) {
                    let result = String::from_utf8_lossy(&pending.message.payload).to_string();
                    let preview = if result.len() > 100 { &result[..100] } else { &result };
                    tracing::info!(
                        job_id = ?job_id,
                        result_len = result.len(),
                        preview = %preview,
                        "Harness received agent response, job completed"
                    );
                    self.registry.update_job_status(&job_id, JobStatus::Completed(result));
                }
            }
            let _ = pending.ack();
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
        ResourceId::new("file:///test/agent.wasm")
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from("tests/fixtures/agents").join(name)
    }

    /// Create a registry with Wasm runtime registered (required for agent deployment).
    fn test_registry() -> Arc<dyn Registry> {
        let registry = InMemoryRegistry::new();
        registry.register_runtime(RuntimeType::Wasm);
        Arc::new(registry)
    }

    /// Deploy a minimal test agent.
    fn deploy_test_agent(harness: &Harness) -> ResourceId {
        let manifest = r#"
            name = "test-agent"
            description = "Test"
            id = "file:///test/agent.wasm"
            [requirements]
            services = []
        "#;
        harness.deploy(manifest).unwrap()
    }

    #[test]
    fn invoke_creates_job_and_queues_message() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = Harness::new(queue.clone(), registry.clone());

        // Deploy agent first
        let agent_id = deploy_test_agent(&harness);

        // Invoke
        let job_id = harness.invoke(&agent_id, "hello").unwrap();

        // Job exists in registry with Running status
        let job = registry.get_job(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.agent_id, agent_id);

        // Message is in queue (keyed by agent_id string)
        let pending = queue.receive(agent_id.as_str()).unwrap();
        assert_eq!(pending.message.payload, b"hello");
        pending.ack().unwrap();
    }

    #[test]
    fn invoke_fails_for_undeployed_agent() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = Harness::new(queue, registry);

        let agent_id = test_agent_id();
        let result = harness.invoke(&agent_id, "hello");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not deployed"));
    }

    #[test]
    fn poll_returns_none_for_running_job() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = Harness::new(queue, registry);

        let agent_id = deploy_test_agent(&harness);
        let job_id = harness.invoke(&agent_id, "hello").unwrap();

        // Poll returns None while job is running
        assert!(harness.poll(&job_id).is_none());
    }

    #[test]
    fn poll_returns_result_for_completed_job() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = Harness::new(queue, registry.clone());

        let agent_id = test_agent_id();
        let job_id = registry.create_job(agent_id, "input".to_string());
        registry.update_job_status(&job_id, JobStatus::Completed("done".to_string()));

        assert_eq!(harness.poll(&job_id), Some("done".to_string()));
    }

    #[test]
    fn tick_reconciles_completed_jobs() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let mut harness = Harness::new(queue.clone(), registry.clone());

        let agent_id = deploy_test_agent(&harness);
        let job_id = harness.invoke(&agent_id, "hello").unwrap();

        // Simulate worker processing: receive request, send response to reply_to
        let pending = queue.receive(agent_id.as_str()).unwrap();
        let response = Message::response(
            b"result".to_vec(),
            &pending.message.reply_to,
            pending.message.id.clone(),
        );
        queue.send(&pending.message.reply_to, response).unwrap();
        pending.ack().unwrap();

        // Before tick: job is still Running
        assert_eq!(registry.get_job(&job_id).unwrap().status, JobStatus::Running);

        // Tick reconciles
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
        let harness = Harness::new(queue, registry.clone());

        let agent_id = harness.deploy_from_path(&fixture_path("echo-agent")).unwrap();

        // Agent is registered
        let agent = registry.get_agent(&agent_id).unwrap();
        assert_eq!(agent.name, "echo-agent");
    }

    #[test]
    fn deploy_from_path_fails_for_nonexistent_path() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = Harness::new(queue, registry);

        let result = harness.deploy_from_path(Path::new("/nonexistent/path"));

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to load agent"));
    }

    #[test]
    fn deploy_from_path_fails_for_path_without_manifest() {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = test_registry();
        let harness = Harness::new(queue, registry);

        // Use tests/ directory which exists but has no agent.toml
        let result = harness.deploy_from_path(Path::new("tests"));

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to load agent"));
    }
}
