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
