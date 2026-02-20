//! Harness - API surface for agent interaction.
//!
//! The harness is the entry point for external requests. Different harness
//! types handle different interfaces (CLI, Web API, WhatsApp, etc.) but share
//! a common contract via the `Harness` trait.
//!
//! Implementations live outside the domain module:
//! - `CoreHarness` — `crate::harness`

use crate::domain::{HarnessType, ResourceId, TimelineId};

/// Common harness operations shared across all harness types.
pub trait Harness {
    /// Identify which transport submitted the job.
    ///
    /// Stamped into every `InvokeMessage` and used by the completion
    /// path to route responses back to the correct consumer.
    fn harness_type(&self) -> HarnessType;

    /// Set the timeline for branch-scoped subjects (ADR 093).
    ///
    /// If `sealed` is true, subsequent invocations will be rejected.
    fn set_timeline(&mut self, timeline: TimelineId, sealed: bool);

    /// Start a conversation session for an agent.
    ///
    /// Creates a session that tracks conversation history, submission
    /// chaining, and state continuity across turns.
    fn start_session(&mut self, agent_name: &str);

    /// Set the initial state for the next invocation.
    ///
    /// Used to resume from a historical state (time travel, session
    /// continuity). The state hash is passed to the agent on the next invoke.
    fn set_initial_state(&mut self, state: String);

    /// Run an agent to completion synchronously.
    ///
    /// Sends input to the agent and blocks until the response arrives.
    /// Returns the agent's output as a string.
    fn run_agent(&mut self, agent_id: &ResourceId, input: &str) -> Result<String, String>;
}
