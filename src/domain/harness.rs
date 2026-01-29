//! Harness - interface between users and the agent system.
//!
//! A harness provides a way to trigger agent execution and receive results.
//! Different harnesses for different contexts: CLI, Web, API, Test.
//!
//! Two core operations:
//! - invoke: tee off an agent run, returns immediately
//! - poll: check for response on reply queue

use std::sync::Arc;

use crate::domain::Agent;
use crate::queue::{InMemoryQueue, Message, MessageId, MessageQueue};
use crate::runtime::WasmRuntime;

/// A harness for interacting with agents.
///
/// Provides the entry point for users to invoke agents and collect results.
/// The harness handles discovery and queue communication.
/// It does NOT execute agents - that's the runtime's job.
pub trait Harness {
    /// Invoke an agent with the given input.
    ///
    /// Returns immediately with a request ID. The agent runs asynchronously.
    fn invoke(&self, agent_name: &str, input: &str) -> Result<MessageId, HarnessError>;

    /// Poll for a response to a previous invocation.
    ///
    /// Returns Some(output) if the response is ready, None if still pending.
    fn poll(&self, request_id: &MessageId) -> Result<Option<String>, HarnessError>;
}

// --- Errors ---

#[derive(Debug)]
pub enum HarnessError {
    /// Agent not found
    AgentNotFound(String),
    /// Failed to send request
    SendFailed(String),
    /// Failed to receive response
    ReceiveFailed(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::AgentNotFound(name) => write!(f, "agent not found: {}", name),
            HarnessError::SendFailed(msg) => write!(f, "send failed: {}", msg),
            HarnessError::ReceiveFailed(msg) => write!(f, "receive failed: {}", msg),
        }
    }
}

impl std::error::Error for HarnessError {}

// --- CliHarness Implementation ---

/// CLI harness for invoking agents from the command line.
///
/// Currently embeds WasmRuntime in-process (no daemon).
/// Will be updated to talk to daemon via socket once that exists.
pub struct CliHarness {
    queue: Arc<InMemoryQueue>,
    runtime: WasmRuntime,
    reply_queue: String,
}

impl CliHarness {
    pub fn new() -> Self {
        let queue = Arc::new(InMemoryQueue::new());
        let runtime = WasmRuntime::new(Arc::clone(&queue));
        let reply_queue = format!("cli-harness-{}", uuid::Uuid::new_v4());
        Self {
            queue,
            runtime,
            reply_queue,
        }
    }

    /// Register an agent to be served by the embedded runtime.
    pub fn register(&mut self, agent: Agent) {
        self.runtime.register(agent);
    }

    /// Run the runtime's tick loop until a response is ready.
    /// This is the embedded mode behavior - blocks until done.
    pub fn run_until_response(&mut self, request_id: &MessageId) -> Result<String, HarnessError> {
        loop {
            self.runtime.tick();
            if let Some(output) = self.poll(request_id)? {
                return Ok(output);
            }
        }
    }
}

impl Harness for CliHarness {
    fn invoke(&self, agent_name: &str, input: &str) -> Result<MessageId, HarnessError> {
        let request = Message::request(input.as_bytes().to_vec(), &self.reply_queue);
        let request_id = request.id.clone();

        self.queue
            .send(agent_name, request)
            .map_err(|e| HarnessError::SendFailed(e.to_string()))?;

        Ok(request_id)
    }

    fn poll(&self, request_id: &MessageId) -> Result<Option<String>, HarnessError> {
        // Try to receive from reply queue
        if let Ok(response) = self.queue.receive(&self.reply_queue) {
            let output = String::from_utf8(response.payload)
                .map_err(|e| HarnessError::ReceiveFailed(e.to_string()))?;

            // Check if this is the one we're looking for
            if response.correlation_id.as_ref() == Some(request_id) {
                return Ok(Some(output));
            }
            // For now, drop mismatched responses (simple single-request model)
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use crate::queue::{InMemoryQueue, Message, MessageQueue, process_one};

    #[test]
    fn invoke_poll_pattern() {
        let queue_system = InMemoryQueue::new();
        let reply_queue = "harness-reply";

        // Harness invokes: send request, get back message ID
        let request = Message::request(b"hello".to_vec(), reply_queue);
        let request_id = request.id.clone();
        queue_system.send("echo-agent", request).unwrap();

        // Worker processes (runtime does this)
        process_one(&queue_system, "echo-agent", |payload| {
            let input = String::from_utf8_lossy(payload);
            format!("echo: {}", input).into_bytes()
        }).unwrap();

        // Harness polls: check reply queue for matching correlation_id
        let response = queue_system.receive(reply_queue).unwrap();
        assert_eq!(response.correlation_id, Some(request_id));

        let output = String::from_utf8(response.payload).unwrap();
        assert_eq!(output, "echo: hello");
    }

    #[test]
    fn fan_out_pattern() {
        let queue_system = InMemoryQueue::new();
        let reply_queue = "harness-reply";

        // Invoke 3 agents (fan-out)
        let req_a = Message::request(b"a".to_vec(), reply_queue);
        let req_b = Message::request(b"b".to_vec(), reply_queue);
        let req_c = Message::request(b"c".to_vec(), reply_queue);

        let id_a = req_a.id.clone();
        let id_b = req_b.id.clone();
        let id_c = req_c.id.clone();

        queue_system.send("upper", req_a).unwrap();
        queue_system.send("upper", req_b).unwrap();
        queue_system.send("upper", req_c).unwrap();

        // Workers process
        for _ in 0..3 {
            process_one(&queue_system, "upper", |p| {
                String::from_utf8_lossy(p).to_uppercase().into_bytes()
            }).unwrap();
        }

        // Collect all responses, match by correlation_id
        let mut results = std::collections::HashMap::new();
        for _ in 0..3 {
            let resp = queue_system.receive(reply_queue).unwrap();
            let output = String::from_utf8(resp.payload).unwrap();
            results.insert(resp.correlation_id.unwrap(), output);
        }

        assert_eq!(results.get(&id_a).unwrap(), "A");
        assert_eq!(results.get(&id_b).unwrap(), "B");
        assert_eq!(results.get(&id_c).unwrap(), "C");
    }

    #[test]
    fn wasm_runtime_pattern() {
        use extism::{Manifest, Plugin, Wasm};

        let queue_system = InMemoryQueue::new();
        let reply_queue = "harness-reply";

        // Harness invokes
        let request = Message::request(b"hello".to_vec(), reply_queue);
        let request_id = request.id.clone();
        queue_system.send("reverse-agent", request).unwrap();

        // WasmRuntime: receive → run WASM → respond
        let wasm_path = "tests/fixtures/agents/reverse-agent/agent.wasm";
        process_one(&queue_system, "reverse-agent", |payload| {
            let wasm = Wasm::file(wasm_path);
            let manifest = Manifest::new([wasm]);
            let mut plugin = Plugin::new(&manifest, [], true).unwrap();
            plugin.call::<_, Vec<u8>>("process", payload).unwrap()
        }).unwrap();

        // Harness polls
        let response = queue_system.receive(reply_queue).unwrap();
        assert_eq!(response.correlation_id, Some(request_id));

        let output = String::from_utf8(response.payload).unwrap();
        assert_eq!(output, "olleh");
    }

    #[test]
    fn cli_harness_end_to_end() {
        use super::{CliHarness, Harness};
        use crate::domain::Agent;
        use std::path::Path;

        // Create harness
        let mut harness = CliHarness::new();

        // Register agent
        let agent = Agent::load(Path::new("tests/fixtures/agents/reverse-agent")).unwrap();
        let agent_name = agent.name.clone();
        harness.register(agent);

        // Invoke and wait for result
        let request_id = harness.invoke(&agent_name, "hello").unwrap();
        let output = harness.run_until_response(&request_id).unwrap();

        assert_eq!(output, "olleh");
    }
}
