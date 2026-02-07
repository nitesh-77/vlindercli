//! NATS-backed message queue with JetStream durability (ADR 044).
//!
//! Provides a sync facade over the async NATS client. The runtime is owned
//! internally, so callers use simple blocking APIs while getting async I/O.
//!
//! Uses typed messages exclusively for full observability.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_nats::jetstream::{self, consumer, stream};
type PullConsumer = async_nats::jetstream::consumer::Consumer<consumer::pull::Config>;
use async_nats::jetstream::message::Message as JetStreamMessage;
use futures::StreamExt;
use tokio::runtime::Runtime;

use super::{
    CompleteMessage, HarnessType, InvokeMessage, MessageId, MessageQueue, QueueError,
    RequestMessage, ResponseMessage, Sequence, SubmissionId,
};
use crate::domain::{ResourceId, RuntimeType};

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
    consumers: Mutex<HashMap<String, PullConsumer>>,
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
                consumers: Mutex::new(HashMap::new()),
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

    /// Escape hatch: access the async client directly.
    pub fn async_client(&self) -> &async_nats::Client {
        &self.inner.client
    }

    /// Escape hatch: access JetStream context directly.
    pub fn jetstream(&self) -> &jetstream::Context {
        &self.inner.jetstream
    }

    /// Get or create a named consumer for the given filter.
    ///
    /// Consumers are cached by filter pattern to avoid creating ephemeral
    /// consumers on every poll — which floods the NATS server and causes
    /// messages to get stuck in "pending ack" limbo on abandoned consumers.
    async fn get_or_create_consumer(&self, filter: &str) -> Result<PullConsumer, QueueError> {
        // Check cache first
        {
            let consumers = self.inner.consumers.lock().unwrap();
            if let Some(consumer) = consumers.get(filter) {
                return Ok(consumer.clone());
            }
        }

        // Create a named consumer — name derived from filter for uniqueness
        let name = filter_to_consumer_name(filter);
        let stream = self.inner.jetstream
            .get_stream("VLINDER")
            .await
            .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

        let consumer = stream
            .create_consumer(consumer::pull::Config {
                name: Some(name),
                filter_subject: filter.to_string(),
                ack_wait: Duration::from_secs(300),
                ..Default::default()
            })
            .await
            .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

        // Cache it
        {
            let mut consumers = self.inner.consumers.lock().unwrap();
            consumers.insert(filter.to_string(), consumer.clone());
        }

        Ok(consumer)
    }

    /// Fetch one message from a subject filter, returning the message and an ack closure.
    async fn fetch_one(&self, filter: &str) -> Result<(JetStreamMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let consumer = self.get_or_create_consumer(filter).await?;

        let mut messages = consumer
            .fetch()
            .max_messages(1)
            .expires(Duration::from_millis(100))
            .messages()
            .await
            .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

        let js_msg = messages
            .next()
            .await
            .ok_or(QueueError::Timeout)?
            .map_err(|e| QueueError::ReceiveFailed(e.to_string()))?;

        // Wrap message for ack closure
        let js_msg_for_ack: Arc<Mutex<Option<JetStreamMessage>>> = Arc::new(Mutex::new(Some(js_msg.clone())));
        let handle = self.inner.runtime.handle().clone();

        let ack_fn: Box<dyn FnOnce() -> Result<(), QueueError> + Send> = Box::new(move || {
            if let Some(msg) = js_msg_for_ack.lock().unwrap().take() {
                handle.block_on(async {
                    msg.ack()
                        .await
                        .map_err(|e| QueueError::ReceiveFailed(format!("ack failed: {}", e)))
                })
            } else {
                Ok(())
            }
        });

        Ok((js_msg, ack_fn))
    }
}

impl MessageQueue for NatsQueue {
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

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("harness", msg.harness.as_str());
            headers.insert("runtime", msg.runtime.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());

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

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            headers.insert("service", msg.service.as_str());
            headers.insert("backend", msg.backend.as_str());
            headers.insert("operation", msg.operation.as_str());
            headers.insert("sequence", msg.sequence.to_string());

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

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            headers.insert("service", msg.service.as_str());
            headers.insert("backend", msg.backend.as_str());
            headers.insert("operation", msg.operation.as_str());
            headers.insert("sequence", msg.sequence.to_string());
            headers.insert("correlation-id", msg.correlation_id.as_str());

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

    fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError> {
        let subject = format!(
            "vlinder.{}.complete.{}.{}",
            msg.submission,
            agent_short_name(&msg.agent_id),
            msg.harness,
        );

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            headers.insert("harness", msg.harness.as_str());

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

    fn receive_invoke(&self, subject_pattern: &str) -> Result<(InvokeMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // Build filter: vlinder.*.invoke.*.*.{agent_pattern}
        let filter = format!("vlinder.*.invoke.*.*.{}", subject_pattern);

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            // Extract typed message from headers
            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let msg = InvokeMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                harness: parse_harness_type(&get_header(headers, "harness")?)?,
                runtime: parse_runtime_type(&get_header(headers, "runtime")?)?,
                agent_id: ResourceId::new(&get_header(headers, "agent-id")?),
                payload: js_msg.payload.to_vec(),
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_request(&self, service: &str, backend: &str, operation: &str) -> Result<(RequestMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // Build filter: vlinder.*.req.*.{service}.{backend}.{operation}.{sequence}
        let filter = format!("vlinder.*.req.*.{}.{}.{}.*", service, backend, operation);

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let msg = RequestMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                agent_id: ResourceId::new(&get_header(headers, "agent-id")?),
                service: get_header(headers, "service")?,
                backend: get_header(headers, "backend")?,
                operation: get_header(headers, "operation")?,
                sequence: Sequence::from(get_header(headers, "sequence")?.parse::<u32>().unwrap_or(1)),
                payload: js_msg.payload.to_vec(),
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_response(&self, request: &RequestMessage) -> Result<(ResponseMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // Build filter from request dimensions
        // Three wildcards match: {agent}.{operation}.{sequence}
        let filter = format!("vlinder.{}.res.{}.{}.*.*.*", request.submission, request.service, request.backend);

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let msg = ResponseMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                agent_id: ResourceId::new(&get_header(headers, "agent-id")?),
                service: get_header(headers, "service")?,
                backend: get_header(headers, "backend")?,
                operation: get_header(headers, "operation")?,
                sequence: Sequence::from(get_header(headers, "sequence")?.parse::<u32>().unwrap_or(1)),
                payload: js_msg.payload.to_vec(),
                correlation_id: MessageId::from(get_header(headers, "correlation-id")?),
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_complete(&self, submission: &SubmissionId, harness: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // Build filter: submission-scoped consumer (ADR 052)
        let filter = format!("vlinder.{}.complete.*.{}", submission, harness);

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let msg = CompleteMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                agent_id: ResourceId::new(&get_header(headers, "agent-id")?),
                harness: parse_harness_type(&get_header(headers, "harness")?)?,
                payload: js_msg.payload.to_vec(),
            };

            Ok((msg, ack_fn))
        })
    }
}

/// Derive a stable consumer name from a filter pattern.
///
/// NATS consumer names must be alphanumeric + dash/underscore.
/// We replace dots and wildcards with underscores.
fn filter_to_consumer_name(filter: &str) -> String {
    filter
        .replace('.', "_")
        .replace('*', "W")
        .replace('>', "G")
}

use super::agent_routing_key as agent_short_name;

/// Extract a header value from NATS headers.
fn get_header(headers: &async_nats::HeaderMap, key: &str) -> Result<String, QueueError> {
    headers
        .get(key)
        .map(|v| v.to_string())
        .ok_or_else(|| QueueError::ReceiveFailed(format!("missing header: {}", key)))
}

/// Parse a harness type string.
fn parse_harness_type(s: &str) -> Result<HarnessType, QueueError> {
    match s {
        "cli" => Ok(HarnessType::Cli),
        "web" => Ok(HarnessType::Web),
        "api" => Ok(HarnessType::Api),
        "whatsapp" => Ok(HarnessType::Whatsapp),
        _ => Err(QueueError::ReceiveFailed(format!("unknown harness type: {}", s))),
    }
}

/// Parse a runtime type string.
fn parse_runtime_type(s: &str) -> Result<RuntimeType, QueueError> {
    match s {
        "container" => Ok(RuntimeType::Container),
        _ => Err(QueueError::ReceiveFailed(format!("unknown runtime type: {}", s))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires running NATS server
    fn connect_to_localhost() {
        let queue = NatsQueue::localhost();
        assert!(queue.is_ok());
    }
}
