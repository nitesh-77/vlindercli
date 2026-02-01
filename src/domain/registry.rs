//! Registry - source of truth for all system state.
//!
//! Stores:
//! - Jobs (submitted, running, completed)
//! - Agent definitions (parsed from TOML)

use std::collections::HashMap;

use crate::domain::{Agent, ResourceId};

/// Unique identifier for a submitted job.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JobId(String);

impl JobId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

/// A submitted job and its current state.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: JobId,
    pub agent_id: ResourceId,
    pub input: String,
    pub status: JobStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Completed(String),
    Failed(String),
}

/// The registry - source of truth for all state.
pub struct Registry {
    /// URI where this registry exposes its API.
    pub id: ResourceId,
    jobs: HashMap<JobId, Job>,
    agents: HashMap<ResourceId, Agent>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            id: ResourceId::new("http://127.0.0.1:9000"),
            jobs: HashMap::new(),
            agents: HashMap::new(),
        }
    }

    // --- Job operations ---

    pub fn create_job(&mut self, agent_id: ResourceId, input: String) -> JobId {
        let id = JobId::new();
        let job = Job {
            id: id.clone(),
            agent_id,
            input,
            status: JobStatus::Pending,
        };
        self.jobs.insert(id.clone(), job);
        id
    }

    pub fn get_job(&self, id: &JobId) -> Option<&Job> {
        self.jobs.get(id)
    }

    pub fn update_job_status(&mut self, id: &JobId, status: JobStatus) {
        if let Some(job) = self.jobs.get_mut(id) {
            job.status = status;
        }
    }

    pub fn pending_jobs(&self) -> Vec<&Job> {
        self.jobs
            .values()
            .filter(|j| j.status == JobStatus::Pending)
            .collect()
    }

    // --- Agent operations ---

    pub fn register_agent(&mut self, agent: Agent) {
        self.agents.insert(agent.id.clone(), agent);
    }

    pub fn get_agent(&self, id: &ResourceId) -> Option<&Agent> {
        self.agents.get(id)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_agent_id() -> ResourceId {
        ResourceId::new("file:///test/agent.wasm")
    }

    #[test]
    fn registry_has_api_endpoint() {
        let registry = Registry::new();

        // Registry exposes an HTTP API endpoint
        assert_eq!(registry.id.scheme(), Some("http"));
        assert!(registry.id.as_str().starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn job_lifecycle() {
        let mut registry = Registry::new();
        let agent_id = test_agent_id();

        // Create job
        let job_id = registry.create_job(agent_id.clone(), "hello".to_string());

        // Initial state is Pending
        let job = registry.get_job(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.agent_id, agent_id);
        assert_eq!(job.input, "hello");

        // Update to Running
        registry.update_job_status(&job_id, JobStatus::Running);
        assert_eq!(registry.get_job(&job_id).unwrap().status, JobStatus::Running);

        // Update to Completed
        registry.update_job_status(&job_id, JobStatus::Completed("result".to_string()));
        assert_eq!(
            registry.get_job(&job_id).unwrap().status,
            JobStatus::Completed("result".to_string())
        );
    }

    #[test]
    fn pending_jobs_filters_by_status() {
        let mut registry = Registry::new();
        let agent_id = test_agent_id();

        let job1 = registry.create_job(agent_id.clone(), "a".to_string());
        let job2 = registry.create_job(agent_id.clone(), "b".to_string());
        let _job3 = registry.create_job(agent_id.clone(), "c".to_string());

        // All three are pending
        assert_eq!(registry.pending_jobs().len(), 3);

        // Mark one as running, one as completed
        registry.update_job_status(&job1, JobStatus::Running);
        registry.update_job_status(&job2, JobStatus::Completed("done".to_string()));

        // Only one pending now
        assert_eq!(registry.pending_jobs().len(), 1);
    }

    #[test]
    fn agent_registration() {
        use std::path::Path;

        let mut registry = Registry::new();
        let fake_id = ResourceId::new("file:///nonexistent/agent.wasm");

        // Agent not found initially
        assert!(registry.get_agent(&fake_id).is_none());

        // Register agent
        let agent = Agent::load(Path::new("tests/fixtures/agents/echo-agent")).unwrap();
        let agent_id = agent.id.clone();
        registry.register_agent(agent);

        // Now found by id
        let agent = registry.get_agent(&agent_id).unwrap();
        assert_eq!(agent.name, "echo-agent");
    }
}
