//! In-memory queue implementation.

use super::{Message, MessageQueue, PendingMessage, QueueError};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// In-memory message queue for single-process use.
///
/// ACK/NACK operations are no-ops since messages are removed from the queue
/// immediately on receive (no durability or redelivery support).
pub struct InMemoryQueue {
    queues: Arc<Mutex<HashMap<String, VecDeque<Message>>>>,
}

impl InMemoryQueue {
    pub fn new() -> Self {
        Self {
            queues: Arc::new(Mutex::new(HashMap::new())),
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
