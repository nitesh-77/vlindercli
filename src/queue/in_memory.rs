//! In-memory queue implementation.

use super::{Message, MessageQueue, QueueError};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// In-memory message queue for single-process use.
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

    fn receive(&self, queue: &str) -> Result<Message, QueueError> {
        let mut queues = self.queues.lock().unwrap();
        let q = queues
            .get_mut(queue)
            .ok_or_else(|| QueueError::QueueNotFound(queue.to_string()))?;

        q.pop_front()
            .ok_or_else(|| QueueError::ReceiveFailed("queue empty".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_receive() {
        let queue = InMemoryQueue::new();

        let msg = Message::new(b"hello".to_vec());
        queue.send("test", msg).unwrap();

        let received = queue.receive("test").unwrap();
        assert_eq!(received.payload, b"hello");
    }

    #[test]
    fn receive_from_empty_queue_fails() {
        let queue = InMemoryQueue::new();
        queue.send("test", Message::new(vec![])).unwrap(); // create queue
        queue.receive("test").unwrap(); // drain it

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

        queue.send("test", Message::new(b"first".to_vec())).unwrap();
        queue.send("test", Message::new(b"second".to_vec())).unwrap();

        let first = queue.receive("test").unwrap();
        let second = queue.receive("test").unwrap();

        assert_eq!(first.payload, b"first");
        assert_eq!(second.payload, b"second");
    }

    #[test]
    fn palindrome_agent_flow() {
        let queue = InMemoryQueue::new();

        // Caller sends "racecar" to palindrome agent, expecting response on "caller" queue
        let request = Message::request(b"racecar".to_vec(), "caller");
        queue.send("palindrome", request).unwrap();

        // Palindrome agent receives
        let received = queue.receive("palindrome").unwrap();
        let input = String::from_utf8(received.payload).unwrap();

        // Agent checks palindrome
        let is_palindrome = input.chars().eq(input.chars().rev());
        let response_payload: &[u8] = if is_palindrome { b"true" } else { b"false" };

        // Agent sends response to reply_to queue
        let reply_to = received.reply_to.expect("request should have reply_to");
        let response = Message::new(response_payload.to_vec());
        queue.send(&reply_to, response).unwrap();

        // Caller receives response
        let result = queue.receive("caller").unwrap();
        assert_eq!(result.payload, b"true");
    }
}
