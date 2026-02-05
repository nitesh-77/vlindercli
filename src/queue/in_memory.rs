//! In-memory queue implementation.

use super::{
    CompleteMessage, InvokeMessage, Message, MessageQueue, ObservableMessage, PendingMessage,
    QueueError, RequestMessage, ResponseMessage,
};
use crate::domain::ResourceId;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// In-memory message queue for single-process use.
///
/// ACK/NACK operations are no-ops since messages are removed from the queue
/// immediately on receive (no durability or redelivery support).
///
/// During ADR 044 migration, maintains dual data structures. Typed send
/// methods write to both, enabling gradual migration of receivers.
pub struct InMemoryQueue {
    queues: Arc<Mutex<HashMap<String, VecDeque<Message>>>>,
    /// Typed queues - pub(crate) for test access
    pub(crate) typed_queues: Arc<Mutex<HashMap<String, VecDeque<ObservableMessage>>>>,
}

impl InMemoryQueue {
    pub fn new() -> Self {
        Self {
            queues: Arc::new(Mutex::new(HashMap::new())),
            typed_queues: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageQueue for InMemoryQueue {
    fn send(&self, queue: &str, msg: Message) -> Result<(), QueueError> {
        let mut queues = self.queues.lock().unwrap();
        queues
            .entry(queue.to_string())
            .or_insert_with(VecDeque::new)
            .push_back(msg);
        Ok(())
    }

    fn receive(&self, queue: &str) -> Result<PendingMessage, QueueError> {
        let mut queues = self.queues.lock().unwrap();
        let q = queues
            .get_mut(queue)
            .ok_or_else(|| QueueError::QueueNotFound(queue.to_string()))?;

        let message = q.pop_front()
            .ok_or_else(|| QueueError::ReceiveFailed("queue empty".to_string()))?;

        // ACK/NACK are no-ops for in-memory queue (message already removed)
        Ok(PendingMessage::new(
            message,
            || Ok(()),  // ack: no-op
            || Ok(()),  // nack: no-op
        ))
    }

    fn service_queue(&self, service: &str, backend: &str, action: &str) -> String {
        if action.is_empty() {
            format!("vlinder.svc.{}.{}", service, backend)
        } else {
            format!("vlinder.svc.{}.{}.{}", service, backend, action)
        }
    }

    fn agent_queue(&self, runtime: &str, agent: &crate::domain::Agent) -> String {
        format!("vlinder.agent.{}.{}", runtime, agent.name)
    }

    fn send_invoke(&self, msg: InvokeMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.invoke.{}.{}.{}",
            msg.submission,
            msg.harness,
            msg.runtime.as_str(),
            agent_short_name(&msg.agent_id),
        );

        // Dual write: typed queue
        {
            let mut typed = self.typed_queues.lock().unwrap();
            typed
                .entry(subject.clone())
                .or_default()
                .push_back(ObservableMessage::Invoke(msg.clone()));
        }

        // Dual write: untyped queue (for existing receivers)
        let untyped = Message {
            id: msg.id,
            payload: msg.payload,
            reply_to: subject.clone(),
            correlation_id: None,
        };
        self.send(&subject, untyped)
    }

    fn send_request(&self, msg: RequestMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.req.{}.{}.{}.{}.{}",
            msg.submission,
            agent_short_name(&msg.agent_id),
            msg.service,
            msg.backend,
            msg.operation,
            msg.sequence,
        );

        // Dual write: typed queue
        {
            let mut typed = self.typed_queues.lock().unwrap();
            typed
                .entry(subject.clone())
                .or_default()
                .push_back(ObservableMessage::Request(msg.clone()));
        }

        // Dual write: untyped queue
        let untyped = Message {
            id: msg.id,
            payload: msg.payload,
            reply_to: subject.clone(),
            correlation_id: None,
        };
        self.send(&subject, untyped)
    }

    fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.res.{}.{}.{}.{}.{}",
            msg.submission,
            msg.service,
            msg.backend,
            agent_short_name(&msg.agent_id),
            msg.operation,
            msg.sequence,
        );

        // Dual write: typed queue
        {
            let mut typed = self.typed_queues.lock().unwrap();
            typed
                .entry(subject.clone())
                .or_default()
                .push_back(ObservableMessage::Response(msg.clone()));
        }

        // Dual write: untyped queue
        let untyped = Message {
            id: msg.id,
            payload: msg.payload,
            reply_to: subject.clone(),
            correlation_id: Some(msg.correlation_id),
        };
        self.send(&subject, untyped)
    }

    fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.complete.{}.{}",
            msg.submission,
            agent_short_name(&msg.agent_id),
            msg.harness,
        );

        // Dual write: typed queue
        {
            let mut typed = self.typed_queues.lock().unwrap();
            typed
                .entry(subject.clone())
                .or_default()
                .push_back(ObservableMessage::Complete(msg.clone()));
        }

        // Dual write: untyped queue
        let untyped = Message {
            id: msg.id,
            payload: msg.payload,
            reply_to: subject.clone(),
            correlation_id: Some(msg.correlation_id),
        };
        self.send(&subject, untyped)
    }

    fn receive_invoke(&self, subject_pattern: &str) -> Result<(InvokeMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        // Find first queue matching pattern with an Invoke message
        for (subject, queue) in typed.iter_mut() {
            if subject.contains(subject_pattern) || subject_pattern.contains("*") {
                if let Some(ObservableMessage::Invoke(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();
                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }

    fn receive_response(&self, subject_pattern: &str) -> Result<(ResponseMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let mut typed = self.typed_queues.lock().unwrap();

        // Find first queue matching pattern with a Response message
        for (subject, queue) in typed.iter_mut() {
            if subject.contains(subject_pattern) || subject_pattern.contains("*") {
                if let Some(ObservableMessage::Response(msg)) = queue.front() {
                    let msg = msg.clone();
                    queue.pop_front();
                    return Ok((msg, Box::new(|| Ok(()))));
                }
            }
        }

        Err(QueueError::Timeout)
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Extract a short name from a ResourceId for queue routing.
///
/// - `file:///path/to/echo-agent.wasm` → "echo-agent"
/// - `memory://test-agent` → "test-agent"
fn agent_short_name(agent_id: &ResourceId) -> String {
    if let Some(path) = agent_id.path() {
        if let Some(filename) = path.rsplit('/').next() {
            let name = filename.strip_suffix(".wasm").unwrap_or(filename);
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    if let Some(authority) = agent_id.authority() {
        return authority.to_string();
    }
    agent_id.as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RuntimeType;
    use crate::queue::{ExpectsReply, HarnessType, Sequence, SubmissionId};

    fn test_agent_id() -> ResourceId {
        ResourceId::new("file:///test/echo-agent.wasm")
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-123".to_string())
    }

    #[test]
    fn send_and_receive() {
        let queue = InMemoryQueue::new();

        let msg = Message::request(b"hello".to_vec(), "reply-queue");
        queue.send("test", msg).unwrap();

        let pending = queue.receive("test").unwrap();
        assert_eq!(pending.message.payload, b"hello");
        pending.ack().unwrap();
    }

    #[test]
    fn receive_from_empty_queue_fails() {
        let queue = InMemoryQueue::new();
        queue.send("test", Message::request(vec![], "reply")).unwrap(); // create queue
        let pending = queue.receive("test").unwrap(); // drain it
        pending.ack().unwrap();

        let result = queue.receive("test");
        assert!(result.is_err());
    }

    #[test]
    fn receive_from_nonexistent_queue_fails() {
        let queue = InMemoryQueue::new();
        let result = queue.receive("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn fifo_order() {
        let queue = InMemoryQueue::new();

        queue.send("test", Message::request(b"first".to_vec(), "reply")).unwrap();
        queue.send("test", Message::request(b"second".to_vec(), "reply")).unwrap();

        let first = queue.receive("test").unwrap();
        let second = queue.receive("test").unwrap();

        assert_eq!(first.message.payload, b"first");
        assert_eq!(second.message.payload, b"second");
        first.ack().unwrap();
        second.ack().unwrap();
    }

    #[test]
    fn palindrome_agent_flow() {
        let queue = InMemoryQueue::new();

        // Caller sends "racecar" to palindrome agent, expecting response on "caller" queue
        let request = Message::request(b"racecar".to_vec(), "caller");
        queue.send("palindrome", request).unwrap();

        // Palindrome agent receives
        let pending = queue.receive("palindrome").unwrap();
        let input = String::from_utf8(pending.message.payload.clone()).unwrap();

        // Agent checks palindrome
        let is_palindrome = input.chars().eq(input.chars().rev());
        let response_payload: &[u8] = if is_palindrome { b"true" } else { b"false" };

        // Agent sends response to reply_to queue
        let reply_to = pending.message.reply_to.clone();
        let response = Message::response(response_payload.to_vec(), &reply_to, pending.message.id.clone());
        queue.send(&reply_to, response).unwrap();
        pending.ack().unwrap();

        // Caller receives response
        let result = queue.receive("caller").unwrap();
        assert_eq!(result.message.payload, b"true");
        result.ack().unwrap();
    }

    #[test]
    fn multiple_requests_with_correlation() {
        let queue = InMemoryQueue::new();

        // Caller sends TWO requests
        let request1 = Message::request(b"racecar".to_vec(), "caller"); // palindrome
        let request2 = Message::request(b"hello".to_vec(), "caller");   // not palindrome

        // Track request IDs for correlation
        let request1_id = request1.id.clone();
        let request2_id = request2.id.clone();

        queue.send("palindrome", request1).unwrap();
        queue.send("palindrome", request2).unwrap();

        // Agent processes both
        for _ in 0..2 {
            let pending = queue.receive("palindrome").unwrap();
            let input = String::from_utf8(pending.message.payload.clone()).unwrap();
            let is_palindrome = input.chars().eq(input.chars().rev());
            let response_payload: &[u8] = if is_palindrome { b"true" } else { b"false" };

            let reply_to = pending.message.reply_to.clone();
            // Response includes correlation_id linking back to request
            let response = Message::response(response_payload.to_vec(), &reply_to, pending.message.id.clone());
            queue.send(&reply_to, response).unwrap();
            pending.ack().unwrap();
        }

        // Caller receives TWO responses
        let resp1 = queue.receive("caller").unwrap();
        let resp2 = queue.receive("caller").unwrap();

        // Now we CAN correlate responses to requests!
        // Collect into a map by correlation_id
        let mut results = std::collections::HashMap::new();
        for pending in [resp1, resp2] {
            let corr_id = pending.message.correlation_id.clone().expect("response should have correlation_id");
            results.insert(corr_id, pending.message.payload.clone());
            pending.ack().unwrap();
        }

        // Match responses to original requests by ID
        assert_eq!(results.get(&request1_id).unwrap(), b"true");  // racecar is palindrome
        assert_eq!(results.get(&request2_id).unwrap(), b"false"); // hello is not
    }

    // ========================================================================
    // Typed message tests (ADR 044)
    // ========================================================================

    #[test]
    fn send_invoke_writes_to_both_queues() {
        let queue = InMemoryQueue::new();

        let invoke = InvokeMessage::new(
            test_submission(),
            HarnessType::Cli,
            RuntimeType::Wasm,
            test_agent_id(),
            b"input".to_vec(),
        );

        queue.send_invoke(invoke).unwrap();

        // Verify message is in typed_queues
        let typed = queue.typed_queues.lock().unwrap();
        assert_eq!(typed.len(), 1);
        let (subject, messages) = typed.iter().next().unwrap();
        assert!(subject.contains("invoke"));
        assert!(subject.contains("cli"));
        assert!(subject.contains("wasm"));
        assert!(subject.contains("echo-agent"));
        assert_eq!(messages.len(), 1);

        // Verify message is also in untyped queues (dual write)
        let untyped = queue.queues.lock().unwrap();
        assert_eq!(untyped.len(), 1);
    }

    #[test]
    fn send_request_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            test_submission(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::first(),
            b"key".to_vec(),
        );

        queue.send_request(request).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        let (subject, _) = typed.iter().next().unwrap();

        // Subject follows ADR 044 pattern
        assert_eq!(subject, "vlinder.sub-test-123.req.echo-agent.kv.sqlite.get.1");
    }

    #[test]
    fn send_response_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let request = RequestMessage::new(
            test_submission(),
            test_agent_id(),
            "kv",
            "sqlite",
            "get",
            Sequence::from(3),
            b"key".to_vec(),
        );
        let response = request.create_reply(b"value".to_vec());

        queue.send_response(response).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        let (subject, _) = typed.iter().next().unwrap();

        // Response subject has service.backend before agent (per ADR 044)
        assert_eq!(subject, "vlinder.sub-test-123.res.kv.sqlite.echo-agent.get.3");
    }

    #[test]
    fn send_complete_builds_correct_subject() {
        let queue = InMemoryQueue::new();

        let invoke = InvokeMessage::new(
            test_submission(),
            HarnessType::Web,
            RuntimeType::Wasm,
            test_agent_id(),
            b"input".to_vec(),
        );
        let complete = invoke.create_reply(b"output".to_vec());

        queue.send_complete(complete).unwrap();

        let typed = queue.typed_queues.lock().unwrap();
        let (subject, _) = typed.iter().next().unwrap();

        assert_eq!(subject, "vlinder.sub-test-123.complete.echo-agent.web");
    }

    #[test]
    fn agent_short_name_extracts_from_file_uri() {
        let id = ResourceId::new("file:///path/to/pensieve.wasm");
        assert_eq!(agent_short_name(&id), "pensieve");
    }

    #[test]
    fn agent_short_name_extracts_from_memory_uri() {
        let id = ResourceId::new("memory://test-agent");
        assert_eq!(agent_short_name(&id), "test-agent");
    }

    // ========================================================================
    // Typed receive tests
    // ========================================================================

    #[test]
    fn receive_invoke_returns_typed_message() {
        let queue = InMemoryQueue::new();

        let invoke = InvokeMessage::new(
            test_submission(),
            HarnessType::Cli,
            RuntimeType::Wasm,
            test_agent_id(),
            b"hello".to_vec(),
        );
        let original_id = invoke.id.clone();

        queue.send_invoke(invoke).unwrap();

        // Receive typed message
        let (received, ack) = queue.receive_invoke("invoke").unwrap();

        assert_eq!(received.id, original_id);
        assert_eq!(received.harness, HarnessType::Cli);
        assert_eq!(received.runtime, RuntimeType::Wasm);
        assert_eq!(received.payload, b"hello");

        ack().unwrap();
    }

    #[test]
    fn receive_invoke_preserves_all_dimensions() {
        let queue = InMemoryQueue::new();

        let submission = test_submission();
        let agent_id = test_agent_id();

        let invoke = InvokeMessage::new(
            submission.clone(),
            HarnessType::Web,
            RuntimeType::Wasm,
            agent_id.clone(),
            b"input".to_vec(),
        );

        queue.send_invoke(invoke).unwrap();

        let (received, _) = queue.receive_invoke("invoke").unwrap();

        // All dimensions preserved for reply construction
        assert_eq!(received.submission, submission);
        assert_eq!(received.agent_id, agent_id);
        assert_eq!(received.harness, HarnessType::Web);
    }
}
