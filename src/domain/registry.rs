//! Registry - source of truth for all system state.
//!
//! Stores:
//! - Jobs (submitted, running, completed)
//! - Agent definitions (parsed from TOML)

use std::collections::HashMap;

use crate::domain::Agent;

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
    pub agent_name: String,
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
    jobs: HashMap<JobId, Job>,
    agents: HashMap<String, Agent>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            agents: HashMap::new(),
        }
    }

    // --- Job operations ---

    pub fn create_job(&mut self, agent_name: String, input: String) -> JobId {
        let id = JobId::new();
        let job = Job {
            id: id.clone(),
            agent_name,
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
        self.agents.insert(agent.name.clone(), agent);
    }

    pub fn get_agent(&self, name: &str) -> Option<&Agent> {
        self.agents.get(name)
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

    #[test]
    fn job_lifecycle() {
        let mut registry = Registry::new();

        // Create job
        let job_id = registry.create_job("test-agent".to_string(), "hello".to_string());

        // Initial state is Pending
        let job = registry.get_job(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.agent_name, "test-agent");
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

        let job1 = registry.create_job("agent".to_string(), "a".to_string());
        let job2 = registry.create_job("agent".to_string(), "b".to_string());
        let _job3 = registry.create_job("agent".to_string(), "c".to_string());

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

        // Agent not found initially
        assert!(registry.get_agent("test-agent").is_none());

        // Register agent
        let agent = Agent::load(Path::new("tests/fixtures/agents/echo-agent")).unwrap();
        registry.register_agent(agent);

        // Now found
        let agent = registry.get_agent("echo-agent").unwrap();
        assert_eq!(agent.name, "echo-agent");
    }
}
