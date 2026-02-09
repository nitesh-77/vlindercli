//! Integration tests that require a running NATS server.

use vlindercli::queue::NatsQueue;

#[test]
#[ignore] // Requires running NATS server
fn connect_to_localhost() {
    let queue = NatsQueue::localhost();
    assert!(queue.is_ok());
}
