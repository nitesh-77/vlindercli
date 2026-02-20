//! Harness - API surface for agent interaction.
//!
//! The harness is the entry point for external requests. Different harness
//! types handle different interfaces (CLI, Web API, WhatsApp, etc.) but share
//! a common contract via the `Harness` trait.
//!
//! Implementations live outside the domain module:
//! - `CliHarness` — `crate::harness`

use crate::domain::registry::JobId;
use crate::domain::ResourceId;

/// Common harness operations shared across all harness types.
pub trait Harness {
    /// The type of this harness (CLI, Web, API, WhatsApp).
    fn harness_type(&self) -> super::HarnessType;

    /// Deploy an agent from TOML manifest content.
    fn deploy(&self, manifest_toml: &str) -> Result<ResourceId, String>;

    /// Submit a job for an already-deployed agent.
    fn invoke(&mut self, agent_id: &ResourceId, input: &str) -> Result<JobId, String>;

    /// Poll for job completion.
    fn poll(&self, job_id: &JobId) -> Option<String>;

    /// Start a conversation session for an agent.
    ///
    /// Creates a session that tracks conversation history, submission
    /// chaining, and state continuity across turns.
    fn start_session(&mut self, agent_name: &str);
}
