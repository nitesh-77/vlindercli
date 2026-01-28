//! Harness - interface between users and the agent system.
//!
//! A harness provides a way to trigger agent execution and receive results.
//! Different harnesses for different contexts: CLI, Web, API, Test.
//!
//! Two core operations:
//! - invoke: tee off an agent run, returns immediately
//! - poll: check for response on reply queue

use crate::queue::MessageId;

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
}
