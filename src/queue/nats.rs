//! NATS-backed message queue with JetStream durability.
//!
//! Provides a sync facade over the async NATS client. The runtime is owned
//! internally, so callers use simple blocking APIs while getting async I/O.

use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::{self, stream, consumer};
use tokio::runtime::Runtime;

use super::{Message, MessageId, MessageQueue, QueueError};

/// NATS queue with JetStream durability.
///
/// Sync facade over async internals. Clone is cheap (Arc).
#[derive(Clone)]
pub struct NatsQueue {
    inner: Arc<NatsQueueInner>,
}

struct NatsQueueInner {
    runtime: Runtime,
    client: async_nats::Client,
    jetstream: jetstream::Context,
}

impl NatsQueue {
    /// Connect to a NATS server.
    ///
    /// Creates the VLINDER stream if it doesn't exist.
    pub fn connect(url: &str) -> Result<Self, QueueError> {
        let runtime = Runtime::new()
            .map_err(|e| QueueError::SendFailed(format!("failed to create runtime: {}", e)))?;

        let (client, jetstream) = runtime.block_on(async {
            let client = async_nats::connect(url)
                .await
                .map_err(|e| QueueError::SendFailed(format!("failed to connect: {}", e)))?;

            let jetstream = jetstream::new(client.clone());

            // Ensure stream exists
            Self::ensure_stream(&jetstream).await?;

            Ok::<_, QueueError>((client, jetstream))
        })?;

        Ok(Self {
            inner: Arc::new(NatsQueueInner {
                runtime,
                client,
                jetstream,
            }),
        })
    }

    /// Connect to localhost NATS (default port 4222).
    pub fn localhost() -> Result<Self, QueueError> {
        Self::connect("nats://localhost:4222")
    }

    /// Ensure the VLINDER stream exists.
    async fn ensure_stream(jetstream: &jetstream::Context) -> Result<(), QueueError> {
        let config = stream::Config {
            name: "VLINDER".to_string(),
            subjects: vec!["vlinder.>".to_string()],
            retention: stream::RetentionPolicy::WorkQueue,
            ..Default::default()
        };

        // get_or_create_stream creates if missing, returns existing if present
        jetstream
            .get_or_create_stream(config)
            .await
            .map_err(|e| QueueError::SendFailed(format!("failed to create stream: {}", e)))?;

        Ok(())
    }

    /// Map internal queue name to NATS subject.
    ///
    /// Queue names include routing info:
    /// - `infer.phi3` → `vlinder.svc.infer.phi3`
    /// - `embed.nomic-embed-text` → `vlinder.svc.embed.nomic-embed-text`
    /// - `kv.default` → `vlinder.svc.kv.default`
    /// - `vec.default` → `vlinder.svc.vec.default`
    /// - `pensieve` (agent name) → `vlinder.agent.pensieve`
    fn to_subject(queue: &str) -> String {
        // Already a full subject
        if queue.starts_with("vlinder.") || queue.starts_with("_INBOX.") {
            return queue.to_string();
        }

        // Parse service.routing_key format
        if let Some((service, routing_key)) = queue.split_once('.') {
            match service {
                "infer" => format!("vlinder.svc.infer.{}", routing_key),
                "embed" => format!("vlinder.svc.embed.{}", routing_key),
                "kv" => format!("vlinder.svc.kv.{}", routing_key),
                "vec" => format!("vlinder.svc.vec.{}", routing_key),
                _ => format!("vlinder.agent.{}", queue), // Unknown service, treat as agent
            }
        } else {
            // No dot = agent name
            format!("vlinder.agent.{}", queue)
        }
    }

    /// Escape hatch: access the async client directly.
    pub fn async_client(&self) -> &async_nats::Client {
        &self.inner.client
    }

    /// Escape hatch: access JetStream context directly.
    pub fn jetstream(&self) -> &jetstream::Context {
        &self.inner.jetstream
    }
}

impl MessageQueue for NatsQueue {
    fn send(&self, queue: &str, msg: Message) -> Result<(), QueueError> {
        let subject = Self::to_subject(queue);

        self.inner.runtime.block_on(async {
            // Build headers with reply_to and correlation_id
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("reply-to", msg.reply_to.as_str());
            headers.insert("msg-id", msg.id.as_str());
            if let Some(ref corr_id) = msg.correlation_id {
                headers.insert("correlation-id", corr_id.as_str());
            }

            // Publish to JetStream
            self.inner
                .jetstream
                .publish_with_headers(subject, headers, msg.payload.into())
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?;

            Ok(())
        })
    }

    fn receive(&self, queue: &str) -> Result<Message, QueueError> {
        let subject = Self::to_subject(queue);

        self.inner.runtime.block_on(async {
            // Create ephemeral pull consumer for this subject
            let stream = self
                .inner
                .jetstream
                .get_stream("VLINDER")
                .await
                .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

            let consumer = stream
                .create_consumer(consumer::pull::Config {
                    filter_subject: subject.clone(),
                    ..Default::default()
                })
                .await
                .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

            // Fetch one message with timeout
            let mut messages = consumer
                .fetch()
                .max_messages(1)
                .expires(Duration::from_secs(30))
                .messages()
                .await
                .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

            // Get the message
            let jetstream_msg = messages
                .next()
                .await
                .ok_or_else(|| QueueError::ReceiveFailed("timeout waiting for message".to_string()))?
                .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

            // Acknowledge receipt
            jetstream_msg
                .ack()
                .await
                .map_err(|e| QueueError::ReceiveFailed(format!("failed to ack: {}", e)))?;

            // Extract headers
            let headers = jetstream_msg.headers.as_ref();
            let reply_to = headers
                .and_then(|h| h.get("reply-to"))
                .map(|v| v.to_string())
                .unwrap_or_default();
            let msg_id = headers
                .and_then(|h| h.get("msg-id"))
                .map(|v| v.to_string())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let correlation_id = headers
                .and_then(|h| h.get("correlation-id"))
                .map(|v| MessageId::from(v.to_string()));

            Ok(Message {
                id: MessageId::from(msg_id),
                payload: jetstream_msg.payload.to_vec(),
                reply_to,
                correlation_id,
            })
        })
    }
}

use futures::StreamExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_subject_maps_services_with_routing() {
        assert_eq!(NatsQueue::to_subject("infer.phi3"), "vlinder.svc.infer.phi3");
        assert_eq!(NatsQueue::to_subject("embed.nomic-embed-text"), "vlinder.svc.embed.nomic-embed-text");
        assert_eq!(NatsQueue::to_subject("kv.default"), "vlinder.svc.kv.default");
        assert_eq!(NatsQueue::to_subject("vec.default"), "vlinder.svc.vec.default");
    }

    #[test]
    fn to_subject_maps_agents() {
        assert_eq!(NatsQueue::to_subject("pensieve"), "vlinder.agent.pensieve");
        assert_eq!(NatsQueue::to_subject("echo"), "vlinder.agent.echo");
    }

    #[test]
    fn to_subject_passes_through_full_subjects() {
        assert_eq!(
            NatsQueue::to_subject("vlinder.custom.thing"),
            "vlinder.custom.thing"
        );
        assert_eq!(
            NatsQueue::to_subject("_INBOX.abc123"),
            "_INBOX.abc123"
        );
    }

    #[test]
    #[ignore] // Requires running NATS server
    fn connect_to_localhost() {
        let queue = NatsQueue::localhost();
        assert!(queue.is_ok());
    }

    #[test]
    #[ignore] // Requires running NATS server
    fn send_and_receive() {
        let queue = NatsQueue::localhost().unwrap();
        let msg = Message::request(b"hello".to_vec(), "test-reply");

        queue.send("infer.phi3", msg).unwrap();

        let received = queue.receive("infer.phi3").unwrap();
        assert_eq!(received.payload, b"hello");
    }
}
