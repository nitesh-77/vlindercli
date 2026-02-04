//! NATS-backed message queue with JetStream durability.
//!
//! Provides a sync facade over the async NATS client. The runtime is owned
//! internally, so callers use simple blocking APIs while getting async I/O.
//!
//! ## PendingMessage Pattern (ADR 043)
//!
//! The `receive()` method returns a `PendingMessage` with deferred ACK/NACK.
//! The JetStream message handle is captured in closures, allowing callers to
//! explicitly acknowledge after successful processing.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_nats::jetstream::{self, stream, consumer};
use async_nats::jetstream::message::Message as JetStreamMessage;
use tokio::runtime::Runtime;

use super::{Message, MessageId, MessageQueue, PendingMessage, QueueError};

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
    /// - `file:///path/to/pensieve.wasm` → `vlinder.agent.pensieve`
    ///
    /// NATS subjects can only contain alphanumeric, `.`, `_`, and `-`.
    /// File URIs are sanitized to extract just the agent name.
    fn to_subject(queue: &str) -> String {
        // Already a full subject
        if queue.starts_with("vlinder.") || queue.starts_with("_INBOX.") {
            return queue.to_string();
        }

        // For file:// URIs, extract just the filename as agent name
        // e.g., "file:///path/to/pensieve.wasm" → "pensieve"
        let sanitized = if queue.starts_with("file://") {
            queue
                .rsplit('/')
                .next()
                .and_then(|f| f.strip_suffix(".wasm"))
                .unwrap_or(queue)
        } else {
            queue
        };

        // Parse service.routing_key format (e.g., infer.phi3, embed.nomic)
        if let Some((service, routing_key)) = sanitized.split_once('.') {
            match service {
                "infer" => format!("vlinder.svc.infer.{}", routing_key),
                "embed" => format!("vlinder.svc.embed.{}", routing_key),
                "kv" => format!("vlinder.svc.kv.{}", routing_key),
                "vec" => format!("vlinder.svc.vec.{}", routing_key),
                _ => format!("vlinder.agent.{}", sanitized), // Unknown service, treat as agent
            }
        } else if let Some((service, action)) = sanitized.split_once('-') {
            // Parse service-action format (e.g., kv-get, kv-put, vec-search, vector-store)
            match service {
                "kv" => format!("vlinder.svc.kv.{}", action),
                "vec" | "vector" => format!("vlinder.svc.vec.{}", action),
                _ => format!("vlinder.agent.{}", sanitized), // Unknown, treat as agent
            }
        } else {
            // Bare service names map to exact subjects (no wildcard - works for both publish and subscribe)
            match sanitized {
                "infer" => "vlinder.svc.infer".to_string(),
                "embed" => "vlinder.svc.embed".to_string(),
                "kv" => "vlinder.svc.kv".to_string(),
                "vec" => "vlinder.svc.vec".to_string(),
                _ => format!("vlinder.agent.{}", sanitized), // Agent name
            }
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

    fn receive(&self, queue: &str) -> Result<PendingMessage, QueueError> {
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

            // Fetch one message with short timeout (workers need to cycle quickly)
            let mut messages = consumer
                .fetch()
                .max_messages(1)
                .expires(Duration::from_millis(100))
                .messages()
                .await
                .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

            // Get the message
            let jetstream_msg = messages
                .next()
                .await
                .ok_or(QueueError::Timeout)?
                .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

            // Extract headers BEFORE capturing jetstream_msg in closure
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

            let message = Message {
                id: MessageId::from(msg_id),
                payload: jetstream_msg.payload.to_vec(),
                reply_to,
                correlation_id,
            };

            // Wrap JetStream message in Arc<Mutex<Option>> so both closures can access it
            // Option allows us to take() the message in whichever closure runs first
            let js_msg: Arc<Mutex<Option<JetStreamMessage>>> = Arc::new(Mutex::new(Some(jetstream_msg)));
            let js_msg_for_ack = Arc::clone(&js_msg);
            let js_msg_for_nack = js_msg;

            // Get runtime handle for async operations in sync closures
            let handle_for_ack = self.inner.runtime.handle().clone();
            let handle_for_nack = self.inner.runtime.handle().clone();

            let ack_fn = move || {
                if let Some(msg) = js_msg_for_ack.lock().unwrap().take() {
                    handle_for_ack.block_on(async {
                        msg.ack()
                            .await
                            .map_err(|e| QueueError::ReceiveFailed(format!("ack failed: {}", e)))
                    })
                } else {
                    // Already ACKed or NACKed
                    Ok(())
                }
            };

            let nack_fn = move || {
                if let Some(msg) = js_msg_for_nack.lock().unwrap().take() {
                    handle_for_nack.block_on(async {
                        use async_nats::jetstream::AckKind;
                        msg.ack_with(AckKind::Nak(None))
                            .await
                            .map_err(|e| QueueError::ReceiveFailed(format!("nack failed: {}", e)))
                    })
                } else {
                    // Already ACKed or NACKed
                    Ok(())
                }
            };

            Ok(PendingMessage::new(message, ack_fn, nack_fn))
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
    fn to_subject_maps_bare_services() {
        // Bare service names map to exact subjects (works for both publish and subscribe)
        assert_eq!(NatsQueue::to_subject("infer"), "vlinder.svc.infer");
        assert_eq!(NatsQueue::to_subject("embed"), "vlinder.svc.embed");
        assert_eq!(NatsQueue::to_subject("kv"), "vlinder.svc.kv");
        assert_eq!(NatsQueue::to_subject("vec"), "vlinder.svc.vec");
    }

    #[test]
    fn to_subject_maps_hyphenated_service_actions() {
        // Storage services use service-action format (e.g., kv-get, vec-search)
        assert_eq!(NatsQueue::to_subject("kv-get"), "vlinder.svc.kv.get");
        assert_eq!(NatsQueue::to_subject("kv-put"), "vlinder.svc.kv.put");
        assert_eq!(NatsQueue::to_subject("vec-search"), "vlinder.svc.vec.search");
        assert_eq!(NatsQueue::to_subject("vec-upsert"), "vlinder.svc.vec.upsert");
        // "vector" is an alias for "vec"
        assert_eq!(NatsQueue::to_subject("vector-store"), "vlinder.svc.vec.store");
        assert_eq!(NatsQueue::to_subject("vector-search"), "vlinder.svc.vec.search");
    }

    #[test]
    fn to_subject_maps_agents() {
        assert_eq!(NatsQueue::to_subject("pensieve"), "vlinder.agent.pensieve");
        assert_eq!(NatsQueue::to_subject("echo"), "vlinder.agent.echo");
    }

    #[test]
    fn to_subject_sanitizes_file_uris() {
        // file:// URIs with invalid NATS characters are sanitized to just the agent name
        assert_eq!(
            NatsQueue::to_subject("file:///Users/foo/agents/pensieve/target/pensieve.wasm"),
            "vlinder.agent.pensieve"
        );
        assert_eq!(
            NatsQueue::to_subject("file:///home/user/echo-agent.wasm"),
            "vlinder.agent.echo-agent"
        );
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

        let pending = queue.receive("infer.phi3").unwrap();
        assert_eq!(pending.message.payload, b"hello");
        pending.ack().unwrap();
    }
}
