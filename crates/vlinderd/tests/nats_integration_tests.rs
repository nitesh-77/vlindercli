//! Integration tests that require a running NATS server.

use vlinderd::queue::NatsQueue;

#[test]
#[ignore] // Run via: just run-integration-tests
fn connect_to_localhost() {
    let queue = NatsQueue::localhost();
    assert!(queue.is_ok());
}
