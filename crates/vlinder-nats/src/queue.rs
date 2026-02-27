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

use vlinder_core::domain::{
    AgentId, CompleteMessage, ContainerDiagnostics, DelegateMessage, DelegateDiagnostics,
    HarnessType, InvokeDiagnostics, InvokeMessage, MessageId, MessageQueue, Nonce,
    Operation, QueueError, RequestDiagnostics, RequestMessage,
    ResponseMessage, RoutingKey, RuntimeType, Sequence, ServiceBackend,
    ServiceDiagnostics, ServiceType, SessionId, SubmissionId, TimelineId,
};

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
            retention: stream::RetentionPolicy::Limits,
            max_age: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
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

        // TODO: inactive_threshold is hardcoded — see ADR 053 for adaptive timeout strategy
        let consumer = stream
            .create_consumer(consumer::pull::Config {
                name: Some(name),
                filter_subject: filter.to_string(),
                ack_wait: Duration::from_secs(300),
                inactive_threshold: Duration::from_secs(300),
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
    ///
    /// If the fetch fails with a 503 (consumer GC'd by server), evicts the
    /// stale consumer from cache, recreates it, and retries once.
    async fn fetch_one(&self, filter: &str) -> Result<(JetStreamMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        match self.try_fetch_one(filter).await {
            Ok(result) => Ok(result),
            Err(QueueError::ReceiveFailed(ref msg)) if msg.contains("503") => {
                tracing::warn!(filter = filter, "consumer stale (503), recreating");
                self.evict_consumer(filter);
                self.try_fetch_one(filter).await
            }
            Err(e) => Err(e),
        }
    }

    /// Attempt a single fetch from the consumer for this filter.
    async fn try_fetch_one(&self, filter: &str) -> Result<(JetStreamMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
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

    /// Remove a cached consumer so the next fetch recreates it.
    fn evict_consumer(&self, filter: &str) {
        let mut consumers = self.inner.consumers.lock().unwrap();
        consumers.remove(filter);
    }
}

impl MessageQueue for NatsQueue {
    fn send_invoke(&self, msg: InvokeMessage) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(&msg.routing_key());

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("timeline-id", msg.timeline.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("session-id", msg.session.as_str());
            headers.insert("harness", msg.harness.as_str());
            headers.insert("runtime", msg.runtime.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            if let Some(ref state) = msg.state {
                headers.insert("state", state.as_str());
            }
            if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
                headers.insert("diagnostics", diag_json.as_str());
            }

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
        let subject = routing_key_to_subject(&msg.routing_key());

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("timeline-id", msg.timeline.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("session-id", msg.session.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            headers.insert("service", msg.service.service_type().as_str());
            headers.insert("backend", msg.service.backend_str());
            headers.insert("operation", msg.operation.as_str());
            headers.insert("sequence", msg.sequence.to_string());
            if let Some(ref state) = msg.state {
                headers.insert("state", state.as_str());
            }
            if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
                headers.insert("diagnostics", diag_json.as_str());
            }

            self.inner
                .jetstream
                .publish_with_headers(subject, headers, msg.payload.clone().into())
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?;

            Ok(())
        })
    }

    fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(&msg.routing_key());

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("timeline-id", msg.timeline.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("session-id", msg.session.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            headers.insert("service", msg.service.service_type().as_str());
            headers.insert("backend", msg.service.backend_str());
            headers.insert("operation", msg.operation.as_str());
            headers.insert("sequence", msg.sequence.to_string());
            headers.insert("correlation-id", msg.correlation_id.as_str());
            headers.insert("status-code", msg.status_code.to_string());
            if let Some(ref state) = msg.state {
                headers.insert("state", state.as_str());
            }
            if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
                headers.insert("diagnostics", diag_json.as_str());
            }

            self.inner
                .jetstream
                .publish_with_headers(subject, headers, msg.payload.clone().into())
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?;

            Ok(())
        })
    }

    fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(&msg.routing_key());

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("timeline-id", msg.timeline.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("session-id", msg.session.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            headers.insert("harness", msg.harness.as_str());
            if let Some(ref state) = msg.state {
                headers.insert("state", state.as_str());
            }
            if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
                headers.insert("diagnostics", diag_json.as_str());
            }

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

    fn receive_invoke(&self, agent: &AgentId) -> Result<(InvokeMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // Build filter: vlinder.{timeline}.{submission}.invoke.{harness}.{runtime}.{agent}
        let filter = format!("vlinder.*.*.invoke.*.*.{}", agent.as_str());

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            // Extract typed message from headers
            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics").ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| InvokeDiagnostics { harness_version: String::new(), history_turns: 0 });

            let msg = InvokeMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                timeline: get_header(headers, "timeline-id").map(TimelineId::from).unwrap_or_else(|_| TimelineId::main()),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::from(get_header(headers, "session-id")?),
                harness: parse_harness_type(&get_header(headers, "harness")?)?,
                runtime: parse_runtime_type(&get_header(headers, "runtime")?)?,
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                payload: js_msg.payload.to_vec(),
                state: get_header(headers, "state").ok(),
                diagnostics,
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_request(&self, service: ServiceBackend, operation: Operation) -> Result<(RequestMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // Build filter: vlinder.{timeline}.{submission}.req.{agent}.{service}.{backend}.{op}.{seq}
        let filter = format!("vlinder.*.*.req.*.{}.{}.{}.*", service.service_type(), service.backend_str(), operation);

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics").ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 });

            let msg = RequestMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                timeline: get_header(headers, "timeline-id").map(TimelineId::from).unwrap_or_else(|_| TimelineId::main()),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::from(get_header(headers, "session-id")?),
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                service: ServiceBackend::from_parts(
                    parse_service_type(&get_header(headers, "service")?)?,
                    &get_header(headers, "backend")?,
                ).ok_or_else(|| QueueError::ReceiveFailed("invalid service/backend".to_string()))?,
                operation: parse_operation(&get_header(headers, "operation")?)?,
                sequence: Sequence::from(get_header(headers, "sequence")?.parse::<u32>().unwrap_or(1)),
                payload: js_msg.payload.to_vec(),
                state: get_header(headers, "state").ok(),
                diagnostics,
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_response(&self, request: &RequestMessage) -> Result<(ResponseMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // Build filter from request dimensions
        // Wildcards match: {timeline}, {agent}.{operation}.{sequence}
        let filter = format!("vlinder.*.{}.res.{}.{}.*.*.*", request.submission, request.service.service_type(), request.service.backend_str());

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics").ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(ServiceDiagnostics::placeholder);

            let status_code = get_header(headers, "status-code").ok()
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(200);

            let msg = ResponseMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                timeline: get_header(headers, "timeline-id").map(TimelineId::from).unwrap_or_else(|_| TimelineId::main()),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::from(get_header(headers, "session-id")?),
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                service: ServiceBackend::from_parts(
                    parse_service_type(&get_header(headers, "service")?)?,
                    &get_header(headers, "backend")?,
                ).ok_or_else(|| QueueError::ReceiveFailed("invalid service/backend".to_string()))?,
                operation: parse_operation(&get_header(headers, "operation")?)?,
                sequence: Sequence::from(get_header(headers, "sequence")?.parse::<u32>().unwrap_or(1)),
                payload: js_msg.payload.to_vec(),
                correlation_id: MessageId::from(get_header(headers, "correlation-id")?),
                state: get_header(headers, "state").ok(),
                diagnostics,
                status_code,
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_complete(&self, submission: &SubmissionId, harness: HarnessType) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // Build filter: submission-scoped consumer (ADR 052) with timeline wildcard
        let filter = format!("vlinder.*.{}.complete.*.{}", submission, harness.as_str());

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics").ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| ContainerDiagnostics::placeholder(0));

            let msg = CompleteMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                timeline: get_header(headers, "timeline-id").map(TimelineId::from).unwrap_or_else(|_| TimelineId::main()),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::from(get_header(headers, "session-id")?),
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                harness: parse_harness_type(&get_header(headers, "harness")?)?,
                payload: js_msg.payload.to_vec(),
                state: get_header(headers, "state").ok(),
                diagnostics,
            };

            Ok((msg, ack_fn))
        })
    }

    fn send_delegate(&self, msg: DelegateMessage) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(&msg.routing_key());

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("timeline-id", msg.timeline.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("session-id", msg.session.as_str());
            headers.insert("caller-agent", msg.caller.as_str());
            headers.insert("target-agent", msg.target.as_str());
            headers.insert("nonce", msg.nonce.as_str());
            if let Some(ref state) = msg.state {
                headers.insert("state", state.as_str());
            }
            if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
                headers.insert("diagnostics", diag_json.as_str());
            }

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

    fn receive_delegate(&self, target: &AgentId) -> Result<(DelegateMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let filter = format!("vlinder.*.*.delegate.*.{}", target.as_str());

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics").ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) });

            let msg = DelegateMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                timeline: get_header(headers, "timeline-id").map(TimelineId::from).unwrap_or_else(|_| TimelineId::main()),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::from(get_header(headers, "session-id")?),
                caller: AgentId::new(get_header(headers, "caller-agent")?),
                target: AgentId::new(get_header(headers, "target-agent")?),
                payload: js_msg.payload.to_vec(),
                nonce: Nonce::new(get_header(headers, "nonce")?),
                state: get_header(headers, "state").ok(),
                diagnostics,
            };

            Ok((msg, ack_fn))
        })
    }

    fn send_delegate_reply(&self, msg: CompleteMessage, reply_key: &RoutingKey) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(reply_key);

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("timeline-id", msg.timeline.as_str());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("session-id", msg.session.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            headers.insert("harness", msg.harness.as_str());
            if let Some(ref state) = msg.state {
                headers.insert("state", state.as_str());
            }
            if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
                headers.insert("diagnostics", diag_json.as_str());
            }

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

    fn receive_delegate_reply(&self, reply_key: &RoutingKey) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        let filter = routing_key_to_subject(reply_key);

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg.headers.as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics").ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| ContainerDiagnostics::placeholder(0));

            let msg = CompleteMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                timeline: get_header(headers, "timeline-id").map(TimelineId::from).unwrap_or_else(|_| TimelineId::main()),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::from(get_header(headers, "session-id")?),
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                harness: parse_harness_type(&get_header(headers, "harness")?)?,
                payload: js_msg.payload.to_vec(),
                state: get_header(headers, "state").ok(),
                diagnostics,
            };

            Ok((msg, ack_fn))
        })
    }
}

/// Serialize a routing key to a NATS subject string (ADR 096 §8).
///
/// This is the single point of truth for RoutingKey → NATS subject
/// serialization. Injectivity: distinct routing keys produce distinct
/// subjects (verified by tests in routing_key.rs).
fn routing_key_to_subject(key: &RoutingKey) -> String {
    match key {
        RoutingKey::Invoke { timeline, submission, harness, runtime, agent } => {
            format!(
                "vlinder.{}.{}.invoke.{}.{}.{}",
                timeline, submission, harness, runtime, agent,
            )
        }
        RoutingKey::Complete { timeline, submission, agent, harness } => {
            format!(
                "vlinder.{}.{}.complete.{}.{}",
                timeline, submission, agent, harness,
            )
        }
        RoutingKey::Request { timeline, submission, agent, service, operation, sequence } => {
            format!(
                "vlinder.{}.{}.req.{}.{}.{}.{}.{}",
                timeline, submission, agent, service.service_type(), service.backend_str(), operation, sequence,
            )
        }
        RoutingKey::Response { timeline, submission, service, agent, operation, sequence } => {
            format!(
                "vlinder.{}.{}.res.{}.{}.{}.{}.{}",
                timeline, submission, service.service_type(), service.backend_str(), agent, operation, sequence,
            )
        }
        RoutingKey::Delegate { timeline, submission, caller, target } => {
            format!(
                "vlinder.{}.{}.delegate.{}.{}",
                timeline, submission, caller, target,
            )
        }
        RoutingKey::DelegateReply { timeline, submission, caller, target, nonce } => {
            format!(
                "vlinder.{}.{}.delegate-reply.{}.{}.{}",
                timeline, submission, caller, target, nonce,
            )
        }
    }
}

/// Parse a NATS subject back into a `RoutingKey`.
///
/// Inverse of `routing_key_to_subject`. Returns `None` for subjects
/// that don't match the `vlinder.<timeline>.<submission>.<type>...` format.
fn subject_to_routing_key(subject: &str) -> Option<RoutingKey> {
    let s: Vec<&str> = subject.split('.').collect();
    if s.len() < 4 || s[0] != "vlinder" {
        return None;
    }

    let timeline = TimelineId::from(s[1].to_string());
    let submission = SubmissionId::from(s[2].to_string());

    match s[3] {
        // vlinder.{timeline}.{submission}.invoke.{harness}.{runtime}.{agent}
        "invoke" if s.len() == 7 => Some(RoutingKey::Invoke {
            timeline,
            submission,
            harness: HarnessType::from_str(s[4])?,
            runtime: RuntimeType::from_str(s[5])?,
            agent: AgentId::new(s[6]),
        }),
        // vlinder.{timeline}.{submission}.req.{agent}.{svc}.{backend}.{op}.{seq}
        "req" if s.len() == 9 => Some(RoutingKey::Request {
            timeline,
            submission,
            agent: AgentId::new(s[4]),
            service: ServiceBackend::from_parts(
                ServiceType::from_str(s[5])?,
                s[6],
            )?,
            operation: Operation::from_str(s[7])?,
            sequence: Sequence::from(s[8].parse::<u32>().ok()?),
        }),
        // vlinder.{timeline}.{submission}.res.{svc}.{backend}.{agent}.{op}.{seq}
        "res" if s.len() == 9 => Some(RoutingKey::Response {
            timeline,
            submission,
            service: ServiceBackend::from_parts(
                ServiceType::from_str(s[4])?,
                s[5],
            )?,
            agent: AgentId::new(s[6]),
            operation: Operation::from_str(s[7])?,
            sequence: Sequence::from(s[8].parse::<u32>().ok()?),
        }),
        // vlinder.{timeline}.{submission}.complete.{agent}.{harness}
        "complete" if s.len() == 6 => Some(RoutingKey::Complete {
            timeline,
            submission,
            agent: AgentId::new(s[4]),
            harness: HarnessType::from_str(s[5])?,
        }),
        // vlinder.{timeline}.{submission}.delegate.{caller}.{target}
        "delegate" if s.len() == 6 => Some(RoutingKey::Delegate {
            timeline,
            submission,
            caller: AgentId::new(s[4]),
            target: AgentId::new(s[5]),
        }),
        // vlinder.{timeline}.{submission}.delegate-reply.{caller}.{target}.{nonce}
        "delegate-reply" if s.len() == 7 => Some(RoutingKey::DelegateReply {
            timeline,
            submission,
            caller: AgentId::new(s[4]),
            target: AgentId::new(s[5]),
            nonce: Nonce::new(s[6]),
        }),
        _ => None,
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
        "grpc" => Ok(HarnessType::Grpc),
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

/// Parse a service type string.
fn parse_service_type(s: &str) -> Result<ServiceType, QueueError> {
    ServiceType::from_str(s)
        .ok_or_else(|| QueueError::ReceiveFailed(format!("unknown service type: {}", s)))
}

/// Parse an operation string.
fn parse_operation(s: &str) -> Result<Operation, QueueError> {
    Operation::from_str(s)
        .ok_or_else(|| QueueError::ReceiveFailed(format!("unknown operation: {}", s)))
}

// ============================================================================
// Tests — subject serialization injectivity (ADR 096 §9)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use vlinder_core::domain::{
        InferenceBackendType, ObjectStorageType, VectorStorageType,
    };

    fn timeline() -> TimelineId { TimelineId::main() }
    fn timeline_alt() -> TimelineId { TimelineId::from(2) }
    fn submission() -> SubmissionId { SubmissionId::from("sub-1".to_string()) }
    fn submission_alt() -> SubmissionId { SubmissionId::from("sub-2".to_string()) }
    fn agent() -> AgentId { AgentId::new("echo") }
    fn agent_alt() -> AgentId { AgentId::new("pensieve") }

    // ========================================================================
    // Format sanity — subjects have the expected shape
    // ========================================================================

    #[test]
    fn invoke_subject_format() {
        let key = RoutingKey::Invoke {
            timeline: timeline(),
            submission: submission(),
            harness: HarnessType::Cli,
            runtime: RuntimeType::Container,
            agent: agent(),
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!("vlinder.{}.{}.invoke.cli.container.echo", timeline(), submission()),
        );
    }

    #[test]
    fn request_subject_format() {
        let key = RoutingKey::Request {
            timeline: timeline(),
            submission: submission(),
            agent: agent(),
            service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
            operation: Operation::Get,
            sequence: Sequence::first(),
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!("vlinder.{}.{}.req.echo.kv.sqlite.get.1", timeline(), submission()),
        );
    }

    #[test]
    fn response_subject_format() {
        let key = RoutingKey::Response {
            timeline: timeline(),
            submission: submission(),
            service: ServiceBackend::Infer(InferenceBackendType::Ollama),
            agent: agent(),
            operation: Operation::Run,
            sequence: Sequence::from(3),
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!("vlinder.{}.{}.res.infer.ollama.echo.run.3", timeline(), submission()),
        );
    }

    #[test]
    fn complete_subject_format() {
        let key = RoutingKey::Complete {
            timeline: timeline(),
            submission: submission(),
            agent: agent(),
            harness: HarnessType::Web,
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!("vlinder.{}.{}.complete.echo.web", timeline(), submission()),
        );
    }

    #[test]
    fn delegate_subject_format() {
        let key = RoutingKey::Delegate {
            timeline: timeline(),
            submission: submission(),
            caller: agent(),
            target: agent_alt(),
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!("vlinder.{}.{}.delegate.echo.pensieve", timeline(), submission()),
        );
    }

    #[test]
    fn delegate_reply_subject_format() {
        let key = RoutingKey::DelegateReply {
            timeline: timeline(),
            submission: submission(),
            caller: agent(),
            target: agent_alt(),
            nonce: Nonce::new("abc123"),
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!("vlinder.{}.{}.delegate-reply.echo.pensieve.abc123", timeline(), submission()),
        );
    }

    // ========================================================================
    // Injectivity — distinct routing keys produce distinct subjects
    // ========================================================================

    /// Helper: assert two different routing keys serialize to different subjects.
    fn assert_injective(a: &RoutingKey, b: &RoutingKey) {
        assert_ne!(a, b, "precondition: keys must differ");
        assert_ne!(
            routing_key_to_subject(a),
            routing_key_to_subject(b),
            "injectivity violated: {a:?} and {b:?} mapped to same subject",
        );
    }

    #[test]
    fn invoke_injective_by_timeline() {
        let a = RoutingKey::Invoke { timeline: timeline(), submission: submission(), harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent() };
        let b = RoutingKey::Invoke { timeline: timeline_alt(), submission: submission(), harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent() };
        assert_injective(&a, &b);
    }

    #[test]
    fn invoke_injective_by_submission() {
        let a = RoutingKey::Invoke { timeline: timeline(), submission: submission(), harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent() };
        let b = RoutingKey::Invoke { timeline: timeline(), submission: submission_alt(), harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent() };
        assert_injective(&a, &b);
    }

    #[test]
    fn invoke_injective_by_harness() {
        let a = RoutingKey::Invoke { timeline: timeline(), submission: submission(), harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent() };
        let b = RoutingKey::Invoke { timeline: timeline(), submission: submission(), harness: HarnessType::Web, runtime: RuntimeType::Container, agent: agent() };
        assert_injective(&a, &b);
    }

    #[test]
    fn invoke_injective_by_agent() {
        let a = RoutingKey::Invoke { timeline: timeline(), submission: submission(), harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent() };
        let b = RoutingKey::Invoke { timeline: timeline(), submission: submission(), harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent_alt() };
        assert_injective(&a, &b);
    }

    #[test]
    fn request_injective_by_service() {
        let a = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Kv(ObjectStorageType::Sqlite), operation: Operation::Get, sequence: Sequence::first() };
        let b = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Vec(VectorStorageType::SqliteVec), operation: Operation::Get, sequence: Sequence::first() };
        assert_injective(&a, &b);
    }

    #[test]
    fn request_injective_by_backend() {
        let a = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Kv(ObjectStorageType::Sqlite), operation: Operation::Get, sequence: Sequence::first() };
        let b = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Kv(ObjectStorageType::InMemory), operation: Operation::Get, sequence: Sequence::first() };
        assert_injective(&a, &b);
    }

    #[test]
    fn request_injective_by_operation() {
        let a = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Kv(ObjectStorageType::Sqlite), operation: Operation::Get, sequence: Sequence::first() };
        let b = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Kv(ObjectStorageType::Sqlite), operation: Operation::Put, sequence: Sequence::first() };
        assert_injective(&a, &b);
    }

    #[test]
    fn request_injective_by_sequence() {
        let a = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Kv(ObjectStorageType::Sqlite), operation: Operation::Get, sequence: Sequence::first() };
        let b = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Kv(ObjectStorageType::Sqlite), operation: Operation::Get, sequence: Sequence::from(2) };
        assert_injective(&a, &b);
    }

    #[test]
    fn response_injective_by_agent() {
        let a = RoutingKey::Response { timeline: timeline(), submission: submission(), service: ServiceBackend::Infer(InferenceBackendType::Ollama), agent: agent(), operation: Operation::Run, sequence: Sequence::first() };
        let b = RoutingKey::Response { timeline: timeline(), submission: submission(), service: ServiceBackend::Infer(InferenceBackendType::Ollama), agent: agent_alt(), operation: Operation::Run, sequence: Sequence::first() };
        assert_injective(&a, &b);
    }

    #[test]
    fn response_injective_by_backend() {
        let a = RoutingKey::Response { timeline: timeline(), submission: submission(), service: ServiceBackend::Infer(InferenceBackendType::Ollama), agent: agent(), operation: Operation::Run, sequence: Sequence::first() };
        let b = RoutingKey::Response { timeline: timeline(), submission: submission(), service: ServiceBackend::Infer(InferenceBackendType::OpenRouter), agent: agent(), operation: Operation::Run, sequence: Sequence::first() };
        assert_injective(&a, &b);
    }

    #[test]
    fn complete_injective_by_harness() {
        let a = RoutingKey::Complete { timeline: timeline(), submission: submission(), agent: agent(), harness: HarnessType::Cli };
        let b = RoutingKey::Complete { timeline: timeline(), submission: submission(), agent: agent(), harness: HarnessType::Web };
        assert_injective(&a, &b);
    }

    #[test]
    fn complete_injective_by_agent() {
        let a = RoutingKey::Complete { timeline: timeline(), submission: submission(), agent: agent(), harness: HarnessType::Cli };
        let b = RoutingKey::Complete { timeline: timeline(), submission: submission(), agent: agent_alt(), harness: HarnessType::Cli };
        assert_injective(&a, &b);
    }

    #[test]
    fn delegate_injective_by_caller() {
        let a = RoutingKey::Delegate { timeline: timeline(), submission: submission(), caller: agent(), target: agent_alt() };
        let b = RoutingKey::Delegate { timeline: timeline(), submission: submission(), caller: agent_alt(), target: agent_alt() };
        assert_injective(&a, &b);
    }

    #[test]
    fn delegate_injective_by_target() {
        let a = RoutingKey::Delegate { timeline: timeline(), submission: submission(), caller: agent(), target: agent_alt() };
        let b = RoutingKey::Delegate { timeline: timeline(), submission: submission(), caller: agent(), target: AgentId::new("fact-checker") };
        assert_injective(&a, &b);
    }

    #[test]
    fn delegate_reply_injective_by_nonce() {
        let a = RoutingKey::DelegateReply { timeline: timeline(), submission: submission(), caller: agent(), target: agent_alt(), nonce: Nonce::new("nonce-1") };
        let b = RoutingKey::DelegateReply { timeline: timeline(), submission: submission(), caller: agent(), target: agent_alt(), nonce: Nonce::new("nonce-2") };
        assert_injective(&a, &b);
    }

    // ========================================================================
    // Cross-variant injectivity — different message types never collide
    // ========================================================================

    #[test]
    fn invoke_and_complete_subjects_differ() {
        let invoke = RoutingKey::Invoke { timeline: timeline(), submission: submission(), harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent() };
        let complete = RoutingKey::Complete { timeline: timeline(), submission: submission(), agent: agent(), harness: HarnessType::Cli };
        assert_injective(&invoke, &complete);
    }

    #[test]
    fn request_and_response_subjects_differ() {
        let request = RoutingKey::Request { timeline: timeline(), submission: submission(), agent: agent(), service: ServiceBackend::Kv(ObjectStorageType::Sqlite), operation: Operation::Get, sequence: Sequence::first() };
        let response = RoutingKey::Response { timeline: timeline(), submission: submission(), service: ServiceBackend::Kv(ObjectStorageType::Sqlite), agent: agent(), operation: Operation::Get, sequence: Sequence::first() };
        assert_injective(&request, &response);
    }

    #[test]
    fn delegate_and_delegate_reply_subjects_differ() {
        let delegate = RoutingKey::Delegate { timeline: timeline(), submission: submission(), caller: agent(), target: agent_alt() };
        let reply = RoutingKey::DelegateReply { timeline: timeline(), submission: submission(), caller: agent(), target: agent_alt(), nonce: Nonce::new("n") };
        assert_injective(&delegate, &reply);
    }

    // ========================================================================
    // Round-trip: routing_key_to_subject → subject_to_routing_key
    // ========================================================================

    fn assert_round_trips(key: &RoutingKey) {
        let subject = routing_key_to_subject(key);
        let recovered = subject_to_routing_key(&subject)
            .unwrap_or_else(|| panic!("failed to parse subject: {}", subject));
        assert_eq!(&recovered, key, "round-trip failed for subject: {}", subject);
    }

    #[test]
    fn invoke_round_trips() {
        assert_round_trips(&RoutingKey::Invoke {
            timeline: timeline(), submission: submission(),
            harness: HarnessType::Cli, runtime: RuntimeType::Container, agent: agent(),
        });
    }

    #[test]
    fn request_round_trips() {
        assert_round_trips(&RoutingKey::Request {
            timeline: timeline(), submission: submission(), agent: agent(),
            service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
            operation: Operation::Get, sequence: Sequence::first(),
        });
    }

    #[test]
    fn response_round_trips() {
        assert_round_trips(&RoutingKey::Response {
            timeline: timeline(), submission: submission(),
            service: ServiceBackend::Infer(InferenceBackendType::Ollama),
            agent: agent(), operation: Operation::Run, sequence: Sequence::from(3),
        });
    }

    #[test]
    fn complete_round_trips() {
        assert_round_trips(&RoutingKey::Complete {
            timeline: timeline(), submission: submission(),
            agent: agent(), harness: HarnessType::Grpc,
        });
    }

    #[test]
    fn delegate_round_trips() {
        assert_round_trips(&RoutingKey::Delegate {
            timeline: timeline(), submission: submission(),
            caller: agent(), target: agent_alt(),
        });
    }

    #[test]
    fn delegate_reply_round_trips() {
        assert_round_trips(&RoutingKey::DelegateReply {
            timeline: timeline(), submission: submission(),
            caller: agent(), target: agent_alt(), nonce: Nonce::new("abc123"),
        });
    }

    #[test]
    fn subject_to_routing_key_rejects_garbage() {
        assert!(subject_to_routing_key("not-a-subject").is_none());
        assert!(subject_to_routing_key("").is_none());
        assert!(subject_to_routing_key("vlinder.1.sub.unknown.stuff").is_none());
    }

    #[test]
    fn subject_to_routing_key_rejects_too_few_segments() {
        assert!(subject_to_routing_key("vlinder.1.sub").is_none());
    }
}

