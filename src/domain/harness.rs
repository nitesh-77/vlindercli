//! Harness - API surface for the system.
//!
//! The harness is the single entry point for external requests:
//! - deploy: Register an agent after validating requirements
//! - invoke: Submit a job for execution
//! - poll: Check job status
//! - tick: Reconcile completed jobs from reply queue

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::domain::registry::{JobId, JobStatus, Registry};
use crate::domain::{Agent, ResourceId};
use crate::queue::{Message, MessageId, MessageQueue};

/// The harness - API surface for agent deployment and job management.
pub struct Harness {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<RwLock<Registry>>,
    reply_queue: String,
    inflight: HashMap<MessageId, JobId>,
}

impl Harness {
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<RwLock<Registry>>,
    ) -> Self {
        Self {
            queue,
            registry,
            reply_queue: format!("harness-{}", uuid::Uuid::new_v4()),
            inflight: HashMap::new(),
        }
    }

    /// Deploy an agent by parsing manifest and registering with Registry.
    ///
    /// Validates all requirements (runtime, storage, models) before registration.
    pub fn deploy(&self, manifest_toml: &str) -> Result<ResourceId, String> {
        let agent = Agent::from_toml(manifest_toml)
            .map_err(|e| format!("failed to parse manifest: {:?}", e))?;

        let agent_id = agent.id.clone();

        let mut registry = self.registry.write().unwrap();
        registry.register_agent(agent)
            .map_err(|e| format!("registration failed: {}", e))?;

        Ok(agent_id)
    }

    /// Submit a job for an already-deployed agent.
    ///
    /// Creates job in registry, queues message, returns job ID.
    pub fn invoke(&mut self, agent_id: &ResourceId, input: &str) -> Result<JobId, String> {
        let mut registry = self.registry.write().unwrap();

        // Verify agent is deployed
        if registry.get_agent(agent_id).is_none() {
            return Err(format!("agent not deployed: {}", agent_id));
        }

        // Create job in registry
        let job_id = registry.create_job(agent_id.clone(), input.to_string());

        // Queue message to agent
        let message = Message::request(input.as_bytes().to_vec(), &self.reply_queue);
        let message_id = message.id.clone();

        self.queue
            .send(agent_id.as_str(), message)
            .map_err(|e| format!("failed to queue: {}", e))?;

        // Track for response correlation
        self.inflight.insert(message_id, job_id.clone());
        registry.update_job_status(&job_id, JobStatus::Running);

        Ok(job_id)
    }

    /// Poll for job completion.
    pub fn poll(&self, job_id: &JobId) -> Option<String> {
        let registry = self.registry.read().unwrap();
        match registry.get_job(job_id)?.status {
            JobStatus::Completed(ref result) => Some(result.clone()),
            JobStatus::Failed(ref error) => Some(format!("[error] {}", error)),
            _ => None,
        }
    }

    /// Tick: monitor reply queue and update completed jobs in registry.
    pub fn tick(&mut self) {
        while let Ok(response) = self.queue.receive(&self.reply_queue) {
            if let Some(correlation_id) = &response.correlation_id {
                if let Some(job_id) = self.inflight.remove(correlation_id) {
                    let result = String::from_utf8_lossy(&response.payload).to_string();
                    let mut registry = self.registry.write().unwrap();
                    registry.update_job_status(&job_id, JobStatus::Completed(result));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RuntimeType;
    use crate::queue::InMemoryQueue;

    fn test_agent_id() -> ResourceId {
        ResourceId::new("file:///test/agent.wasm")
    }

    /// Create a registry with Wasm runtime registered (required for agent deployment).
    fn test_registry() -> Arc<RwLock<Registry>> {
        let mut registry = Registry::new();
        registry.register_runtime(RuntimeType::Wasm);
        Arc::new(RwLock::new(registry))
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
        let reg = registry.read().unwrap();
        let job = reg.get_job(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.agent_id, agent_id);

        // Message is in queue (keyed by agent_id string)
        let msg = queue.receive(agent_id.as_str()).unwrap();
        assert_eq!(msg.payload, b"hello");
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
        let job_id = {
            let mut reg = registry.write().unwrap();
            let job_id = reg.create_job(agent_id, "input".to_string());
            reg.update_job_status(&job_id, JobStatus::Completed("done".to_string()));
            job_id
        };

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
        let request = queue.receive(agent_id.as_str()).unwrap();
        let response = Message::response(
            b"result".to_vec(),
            &request.reply_to,
            request.id.clone(),
        );
        queue.send(&request.reply_to, response).unwrap();

        // Before tick: job is still Running
        {
            let reg = registry.read().unwrap();
            assert_eq!(reg.get_job(&job_id).unwrap().status, JobStatus::Running);
        }

        // Tick reconciles
        harness.tick();

        // After tick: job is Completed
        {
            let reg = registry.read().unwrap();
            assert_eq!(
                reg.get_job(&job_id).unwrap().status,
                JobStatus::Completed("result".to_string())
            );
        }
    }
}
