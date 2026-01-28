//! Worker abstraction for queue consumers.
//!
//! A worker listens to a queue, processes messages, and sends responses.
//! This pattern is used by agents, services, and the runtime.

use super::{Message, MessageQueue, QueueError};

/// Process a single message from a queue and send the response.
///
/// This is the core queue consumer pattern:
/// 1. Receive message from queue
/// 2. Process payload with handler
/// 3. Send response to reply_to queue
pub fn process_one<Q, F>(
    queue: &Q,
    queue_name: &str,
    handler: F,
) -> Result<(), WorkerError>
where
    Q: MessageQueue,
    F: FnOnce(&[u8]) -> Vec<u8>,
{
    // Receive
    let request = queue.receive(queue_name).map_err(WorkerError::Receive)?;

    // Process
    let response_payload = handler(&request.payload);

    // Respond
    let response = Message::response(
        response_payload,
        &request.reply_to,
        request.id.clone(),
    );
    queue.send(&request.reply_to, response).map_err(WorkerError::Send)?;

    Ok(())
}

// --- Errors ---

#[derive(Debug)]
pub enum WorkerError {
    Receive(QueueError),
    Send(QueueError),
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerError::Receive(e) => write!(f, "failed to receive: {}", e),
            WorkerError::Send(e) => write!(f, "failed to send response: {}", e),
        }
    }
}

impl std::error::Error for WorkerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::InMemoryQueue;

    #[test]
    fn process_one_palindrome() {
        let queue = InMemoryQueue::new();

        // Caller sends request
        let request = Message::request(b"racecar".to_vec(), "caller");
        queue.send("palindrome", request).unwrap();

        // Worker processes one message
        process_one(&queue, "palindrome", |payload| {
            let input = String::from_utf8_lossy(payload);
            let is_palindrome = input.chars().eq(input.chars().rev());
            if is_palindrome { b"true".to_vec() } else { b"false".to_vec() }
        }).unwrap();

        // Caller receives response
        let response = queue.receive("caller").unwrap();
        assert_eq!(response.payload, b"true");
    }

    #[test]
    fn process_multiple_messages() {
        let queue = InMemoryQueue::new();

        // Send 3 requests
        queue.send("echo", Message::request(b"one".to_vec(), "caller")).unwrap();
        queue.send("echo", Message::request(b"two".to_vec(), "caller")).unwrap();
        queue.send("echo", Message::request(b"three".to_vec(), "caller")).unwrap();

        // Worker processes each one
        for _ in 0..3 {
            process_one(&queue, "echo", |payload| payload.to_vec()).unwrap();
        }

        // All responses received
        assert_eq!(queue.receive("caller").unwrap().payload, b"one");
        assert_eq!(queue.receive("caller").unwrap().payload, b"two");
        assert_eq!(queue.receive("caller").unwrap().payload, b"three");
    }
}
