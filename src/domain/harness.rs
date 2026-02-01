//! Harness - API surface owned by Daemon.
//!
//! The harness is an endpoint that:
//! - Accepts invoke/poll requests
//! - Stores jobs in Registry
//! - Monitors its reply queue
//! - Updates completed job status in Registry

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::registry::{JobId, JobStatus, Registry};
use crate::domain::ResourceId;
use crate::queue::{Message, MessageId, MessageQueue};

/// The harness - API surface for job submission and status.
pub struct Harness {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    reply_queue: String,
    inflight: HashMap<MessageId, JobId>,
}

impl Harness {
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>) -> Self {
        Self {
            queue,
            reply_queue: format!("harness-{}", uuid::Uuid::new_v4()),
            inflight: HashMap::new(),
        }
    }

    /// Submit a job for an agent.
    ///
    /// Creates job in registry, queues message, returns job ID.
    pub fn invoke(
        &mut self,
        registry: &mut Registry,
        agent_id: &ResourceId,
        input: &str,
    ) -> Result<JobId, String> {
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
    pub fn poll(&self, registry: &Registry, job_id: &JobId) -> Option<String> {
        match registry.get_job(job_id)?.status {
            JobStatus::Completed(ref result) => Some(result.clone()),
            JobStatus::Failed(ref error) => Some(format!("[error] {}", error)),
            _ => None,
        }
    }

    /// Tick: monitor reply queue and update completed jobs in registry.
    pub fn tick(&mut self, registry: &mut Registry) {
        while let Ok(response) = self.queue.receive(&self.reply_queue) {
            if let Some(correlation_id) = &response.correlation_id {
                if let Some(job_id) = self.inflight.remove(correlation_id) {
                    let result = String::from_utf8_lossy(&response.payload).to_string();
                    registry.update_job_status(&job_id, JobStatus::Completed(result));
                }
            }
        }
    }

    pub fn reply_queue(&self) -> &str {
        &self.reply_queue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::InMemoryQueue;

    fn test_agent_id() -> ResourceId {
        ResourceId::new("file:///test/agent.wasm")
    }

    #[test]
    fn invoke_creates_job_and_queues_message() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut harness = Harness::new(queue.clone());
        let mut registry = Registry::new();
        let agent_id = test_agent_id();

        let job_id = harness.invoke(&mut registry, &agent_id, "hello").unwrap();

        // Job exists in registry with Running status
        let job = registry.get_job(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.agent_id, agent_id);

        // Message is in queue (keyed by agent_id string)
        let msg = queue.receive(agent_id.as_str()).unwrap();
        assert_eq!(msg.payload, b"hello");
    }

    #[test]
    fn poll_returns_none_for_running_job() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut harness = Harness::new(queue.clone());
        let mut registry = Registry::new();
        let agent_id = test_agent_id();

        let job_id = harness.invoke(&mut registry, &agent_id, "hello").unwrap();

        // Poll returns None while job is running
        assert!(harness.poll(&registry, &job_id).is_none());
    }

    #[test]
    fn poll_returns_result_for_completed_job() {
        let queue = Arc::new(InMemoryQueue::new());
        let harness = Harness::new(queue);
        let mut registry = Registry::new();
        let agent_id = test_agent_id();

        let job_id = registry.create_job(agent_id, "input".to_string());
        registry.update_job_status(&job_id, JobStatus::Completed("done".to_string()));

        assert_eq!(harness.poll(&registry, &job_id), Some("done".to_string()));
    }

    #[test]
    fn tick_reconciles_completed_jobs() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut harness = Harness::new(queue.clone());
        let mut registry = Registry::new();
        let agent_id = test_agent_id();

        // Invoke creates job and queues message
        let job_id = harness.invoke(&mut registry, &agent_id, "hello").unwrap();

        // Simulate worker processing: receive request, send response
        let request = queue.receive(agent_id.as_str()).unwrap();
        let response = Message::response(
            b"result".to_vec(),
            harness.reply_queue(),
            request.id.clone(),
        );
        queue.send(harness.reply_queue(), response).unwrap();

        // Before tick: job is still Running
        assert_eq!(registry.get_job(&job_id).unwrap().status, JobStatus::Running);

        // Tick reconciles
        harness.tick(&mut registry);

        // After tick: job is Completed
        assert_eq!(
            registry.get_job(&job_id).unwrap().status,
            JobStatus::Completed("result".to_string())
        );
    }
}
