//! NATS-backed message queue with `JetStream` durability (ADR 044).
//!
//! Provides a sync facade over the async NATS client. The runtime is owned
//! internally, so callers use simple blocking APIs while getting async I/O.
//!
//! Uses typed messages exclusively for full observability.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_nats::jetstream::{self, consumer, stream};
type PullConsumer = async_nats::jetstream::consumer::Consumer<consumer::pull::Config>;
use async_nats::jetstream::message::Message as JetStreamMessage;
use futures::StreamExt;
use tokio::runtime::Runtime;

use std::str::FromStr;

use vlinder_core::domain::{
    Acknowledgement, AgentId, BranchId, CompleteMessage, DagNodeId, DelegateDiagnostics,
    DelegateMessage, HarnessType, InvokeDiagnostics, InvokeMessage, MessageId, MessageQueue, Nonce,
    Operation, QueueError, RepairMessage, RequestDiagnostics, RequestMessage, ResponseMessage,
    RoutingKey, RoutingKind, RuntimeDiagnostics, RuntimeType, Sequence, ServiceBackend,
    ServiceDiagnostics, ServiceType, SessionId, SubmissionId,
};

/// NATS queue with `JetStream` durability.
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
    /// Connect to a NATS server using the given config.
    ///
    /// Creates the VLINDER stream if it doesn't exist.
    pub fn connect(config: &crate::NatsConfig) -> Result<Self, QueueError> {
        let runtime = Runtime::new()
            .map_err(|e| QueueError::SendFailed(format!("failed to create runtime: {e}")))?;

        let (client, jetstream) = runtime.block_on(async {
            let client = crate::connect::nats_connect(config)
                .await
                .map_err(QueueError::SendFailed)?;

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

    /// Ensure the VLINDER stream exists.
    async fn ensure_stream(jetstream: &jetstream::Context) -> Result<(), QueueError> {
        let config = stream::Config {
            name: "VLINDER".to_string(),
            subjects: vec!["vlinder.>".to_string()],
            retention: stream::RetentionPolicy::Limits,
            max_age: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
            max_bytes: 100 * 1024 * 1024,                   // 100 MiB — required by NGS
            ..Default::default()
        };

        // get_or_create_stream creates if missing, returns existing if present
        jetstream
            .get_or_create_stream(config)
            .await
            .map_err(|e| QueueError::SendFailed(format!("failed to create stream: {e}")))?;

        Ok(())
    }

    /// Escape hatch: access the async client directly.
    pub fn async_client(&self) -> &async_nats::Client {
        &self.inner.client
    }

    /// Escape hatch: access `JetStream` context directly.
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
        let stream = self
            .inner
            .jetstream
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
    async fn fetch_one(
        &self,
        filter: &str,
    ) -> Result<
        (
            JetStreamMessage,
            Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
        ),
        QueueError,
    > {
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
    async fn try_fetch_one(
        &self,
        filter: &str,
    ) -> Result<
        (
            JetStreamMessage,
            Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
        ),
        QueueError,
    > {
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
        let js_msg_for_ack: Arc<Mutex<Option<JetStreamMessage>>> =
            Arc::new(Mutex::new(Some(js_msg.clone())));
        let handle = self.inner.runtime.handle().clone();

        let ack_fn: Box<dyn FnOnce() -> Result<(), QueueError> + Send> = Box::new(move || {
            if let Some(msg) = js_msg_for_ack.lock().unwrap().take() {
                handle.block_on(async {
                    msg.ack()
                        .await
                        .map_err(|e| QueueError::ReceiveFailed(format!("ack failed: {e}")))
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
            headers.insert("branch-id", msg.branch.to_string());
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
            if !msg.dag_parent.is_empty() {
                headers.insert("dag-parent", msg.dag_parent.as_str());
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
            headers.insert("branch-id", msg.branch.to_string());
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
            if let Some(ref checkpoint) = msg.checkpoint {
                headers.insert("checkpoint", checkpoint.as_str());
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
            headers.insert("branch-id", msg.branch.to_string());
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
            if let Some(ref checkpoint) = msg.checkpoint {
                headers.insert("checkpoint", checkpoint.as_str());
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
            headers.insert("branch-id", msg.branch.to_string());
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

    fn receive_invoke(
        &self,
        agent: &AgentId,
    ) -> Result<(InvokeMessage, Acknowledgement), QueueError> {
        // Build filter: vlinder.{session}.{branch}.{submission}.invoke.{harness}.{runtime}.{agent}
        let filter = format!("vlinder.*.*.*.invoke.*.*.{}", agent.as_str());

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            // Extract typed message from headers
            let headers = js_msg
                .headers
                .as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| InvokeDiagnostics {
                    harness_version: String::new(),
                });

            let msg = InvokeMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                branch: get_header(headers, "branch-id")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map_or(BranchId::from(1), BranchId::from),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::try_from(get_header(headers, "session-id")?)
                    .map_err(QueueError::ReceiveFailed)?,
                harness: HarnessType::from_str(&get_header(headers, "harness")?)
                    .map_err(|_| QueueError::ReceiveFailed("unknown harness type".to_string()))?,
                runtime: RuntimeType::from_str(&get_header(headers, "runtime")?)
                    .map_err(|_| QueueError::ReceiveFailed("unknown runtime type".to_string()))?,
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                payload: js_msg.payload.to_vec(),
                state: get_header(headers, "state").ok(),
                diagnostics,
                dag_parent: DagNodeId::from(get_header(headers, "dag-parent").unwrap_or_default()),
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_request(
        &self,
        service: ServiceBackend,
        operation: Operation,
    ) -> Result<(RequestMessage, Acknowledgement), QueueError> {
        // Build filter: vlinder.{session}.{branch}.{submission}.req.{agent}.{service}.{backend}.{op}.{seq}
        let filter = format!(
            "vlinder.*.*.*.req.*.{}.{}.{}.*",
            service.service_type(),
            service.backend_str(),
            operation
        );

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg
                .headers
                .as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| RequestDiagnostics {
                    sequence: 0,
                    endpoint: String::new(),
                    request_bytes: 0,
                    received_at_ms: 0,
                });

            let msg = RequestMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                branch: get_header(headers, "branch-id")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map_or(BranchId::from(1), BranchId::from),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::try_from(get_header(headers, "session-id")?)
                    .map_err(QueueError::ReceiveFailed)?,
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                service: ServiceBackend::from_parts(
                    ServiceType::from_str(&get_header(headers, "service")?).map_err(|_| {
                        QueueError::ReceiveFailed("unknown service type".to_string())
                    })?,
                    &get_header(headers, "backend")?,
                )
                .ok_or_else(|| QueueError::ReceiveFailed("invalid service/backend".to_string()))?,
                operation: Operation::from_str(&get_header(headers, "operation")?)
                    .map_err(|_| QueueError::ReceiveFailed("unknown operation".to_string()))?,
                sequence: Sequence::from(
                    get_header(headers, "sequence")?.parse::<u32>().unwrap_or(1),
                ),
                payload: js_msg.payload.to_vec(),
                state: get_header(headers, "state").ok(),
                diagnostics,
                checkpoint: get_header(headers, "checkpoint").ok(),
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_response(
        &self,
        request: &RequestMessage,
    ) -> Result<(ResponseMessage, Acknowledgement), QueueError> {
        // Build filter from request dimensions — session and branch pinned (ADR 117).
        let filter = format!(
            "vlinder.{}.{}.{}.res.{}.{}.*.*.*",
            request.session,
            request.branch,
            request.submission,
            request.service.service_type(),
            request.service.backend_str()
        );

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg
                .headers
                .as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(ServiceDiagnostics::placeholder);

            let status_code = get_header(headers, "status-code")
                .ok()
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(200);

            let msg = ResponseMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                branch: get_header(headers, "branch-id")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map_or(BranchId::from(1), BranchId::from),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::try_from(get_header(headers, "session-id")?)
                    .map_err(QueueError::ReceiveFailed)?,
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                service: ServiceBackend::from_parts(
                    ServiceType::from_str(&get_header(headers, "service")?).map_err(|_| {
                        QueueError::ReceiveFailed("unknown service type".to_string())
                    })?,
                    &get_header(headers, "backend")?,
                )
                .ok_or_else(|| QueueError::ReceiveFailed("invalid service/backend".to_string()))?,
                operation: Operation::from_str(&get_header(headers, "operation")?)
                    .map_err(|_| QueueError::ReceiveFailed("unknown operation".to_string()))?,
                sequence: Sequence::from(
                    get_header(headers, "sequence")?.parse::<u32>().unwrap_or(1),
                ),
                payload: js_msg.payload.to_vec(),
                correlation_id: MessageId::from(get_header(headers, "correlation-id")?),
                state: get_header(headers, "state").ok(),
                diagnostics,
                status_code,
                checkpoint: get_header(headers, "checkpoint").ok(),
            };

            Ok((msg, ack_fn))
        })
    }

    fn receive_complete(
        &self,
        submission: &SubmissionId,
        harness: HarnessType,
    ) -> Result<(CompleteMessage, Acknowledgement), QueueError> {
        // Build filter: vlinder.{session}.{branch}.{submission}.complete.{agent}.{harness}
        let filter = format!("vlinder.*.*.{}.complete.*.{}", submission, harness.as_str());

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg
                .headers
                .as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| RuntimeDiagnostics::placeholder(0));

            let msg = CompleteMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                branch: get_header(headers, "branch-id")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map_or(BranchId::from(1), BranchId::from),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::try_from(get_header(headers, "session-id")?)
                    .map_err(QueueError::ReceiveFailed)?,
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                harness: HarnessType::from_str(&get_header(headers, "harness")?)
                    .map_err(|_| QueueError::ReceiveFailed("unknown harness type".to_string()))?,
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
            headers.insert("branch-id", msg.branch.to_string());
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

    fn receive_delegate(
        &self,
        target: &AgentId,
    ) -> Result<(DelegateMessage, Acknowledgement), QueueError> {
        let filter = format!("vlinder.*.*.*.delegate.*.{}", target.as_str());

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg
                .headers
                .as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| DelegateDiagnostics {
                    runtime: RuntimeDiagnostics::placeholder(0),
                });

            let msg = DelegateMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                branch: get_header(headers, "branch-id")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map_or(BranchId::from(1), BranchId::from),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::try_from(get_header(headers, "session-id")?)
                    .map_err(QueueError::ReceiveFailed)?,
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

    fn send_delegate_reply(
        &self,
        msg: CompleteMessage,
        reply_key: &RoutingKey,
    ) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(reply_key);

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("branch-id", msg.branch.to_string());
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

    fn receive_delegate_reply(
        &self,
        reply_key: &RoutingKey,
    ) -> Result<(CompleteMessage, Acknowledgement), QueueError> {
        let filter = routing_key_to_subject(reply_key);

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg
                .headers
                .as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let diagnostics = get_header(headers, "diagnostics")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| RuntimeDiagnostics::placeholder(0));

            let msg = CompleteMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                branch: get_header(headers, "branch-id")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map_or(BranchId::from(1), BranchId::from),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::try_from(get_header(headers, "session-id")?)
                    .map_err(QueueError::ReceiveFailed)?,
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                harness: HarnessType::from_str(&get_header(headers, "harness")?)
                    .map_err(|_| QueueError::ReceiveFailed("unknown harness type".to_string()))?,
                payload: js_msg.payload.to_vec(),
                state: get_header(headers, "state").ok(),
                diagnostics,
            };

            Ok((msg, ack_fn))
        })
    }

    fn send_repair(&self, msg: RepairMessage) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(&msg.routing_key());

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("branch-id", msg.branch.to_string());
            headers.insert("submission-id", msg.submission.as_str());
            headers.insert("session-id", msg.session.as_str());
            headers.insert("agent-id", msg.agent_id.as_str());
            headers.insert("harness", msg.harness.as_str());
            headers.insert("dag-parent", msg.dag_parent.as_str());
            headers.insert("checkpoint", msg.checkpoint.as_str());
            headers.insert("service", msg.service.service_type().as_str());
            headers.insert("backend", msg.service.backend_str());
            headers.insert("operation", msg.operation.as_str());
            headers.insert("sequence", msg.sequence.to_string());
            if let Some(ref state) = msg.state {
                headers.insert("state", state.as_str());
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

    fn receive_repair(
        &self,
        agent: &AgentId,
    ) -> Result<(RepairMessage, Acknowledgement), QueueError> {
        // Build filter: vlinder.{session}.{branch}.{submission}.repair.{harness}.{agent}
        let filter = format!("vlinder.*.*.*.repair.*.{}", agent.as_str());

        self.inner.runtime.block_on(async {
            let (js_msg, ack_fn) = self.fetch_one(&filter).await?;

            let headers = js_msg
                .headers
                .as_ref()
                .ok_or_else(|| QueueError::ReceiveFailed("missing headers".to_string()))?;

            let msg = RepairMessage {
                id: MessageId::from(get_header(headers, "msg-id")?),
                protocol_version: get_header(headers, "protocol-version").unwrap_or_default(),
                branch: get_header(headers, "branch-id")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map_or(BranchId::from(1), BranchId::from),
                submission: SubmissionId::from(get_header(headers, "submission-id")?),
                session: SessionId::try_from(get_header(headers, "session-id")?)
                    .map_err(QueueError::ReceiveFailed)?,
                agent_id: AgentId::new(get_header(headers, "agent-id")?),
                harness: HarnessType::from_str(&get_header(headers, "harness")?)
                    .map_err(|_| QueueError::ReceiveFailed("unknown harness type".to_string()))?,
                dag_parent: DagNodeId::from(get_header(headers, "dag-parent")?),
                checkpoint: get_header(headers, "checkpoint")?,
                service: ServiceBackend::from_parts(
                    ServiceType::from_str(&get_header(headers, "service")?).map_err(|_| {
                        QueueError::ReceiveFailed("unknown service type".to_string())
                    })?,
                    &get_header(headers, "backend")?,
                )
                .ok_or_else(|| QueueError::ReceiveFailed("invalid service/backend".to_string()))?,
                operation: Operation::from_str(&get_header(headers, "operation")?)
                    .map_err(|_| QueueError::ReceiveFailed("unknown operation".to_string()))?,
                sequence: Sequence::from(
                    get_header(headers, "sequence")?.parse::<u32>().unwrap_or(1),
                ),
                payload: js_msg.payload.to_vec(),
                state: get_header(headers, "state").ok(),
            };

            Ok((msg, ack_fn))
        })
    }

    fn send_fork(&self, msg: vlinder_core::domain::ForkMessage) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(&RoutingKey {
            session: msg.session.clone(),
            branch: msg.branch,
            submission: msg.submission.clone(),
            kind: RoutingKind::Fork {
                agent_name: msg.agent_name.clone(),
            },
        });

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("session-id", msg.session.as_str());
            headers.insert("branch-name", msg.branch_name.as_str());
            headers.insert("fork-point", msg.fork_point.as_str());

            self.inner
                .jetstream
                .publish_with_headers(subject, headers, "".into())
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?;

            Ok(())
        })
    }

    fn send_promote(&self, msg: vlinder_core::domain::PromoteMessage) -> Result<(), QueueError> {
        let subject = routing_key_to_subject(&RoutingKey {
            session: msg.session.clone(),
            branch: msg.branch,
            submission: msg.submission.clone(),
            kind: RoutingKind::Promote {
                agent_name: msg.agent_name.clone(),
            },
        });

        self.inner.runtime.block_on(async {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("msg-id", msg.id.as_str());
            headers.insert("protocol-version", msg.protocol_version.as_str());
            headers.insert("session-id", msg.session.as_str());

            self.inner
                .jetstream
                .publish_with_headers(subject, headers, "".into())
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?
                .await
                .map_err(|e| QueueError::SendFailed(e.to_string()))?;

            Ok(())
        })
    }

    fn send_session_start(
        &self,
        _msg: vlinder_core::domain::SessionStartMessage,
    ) -> Result<vlinder_core::domain::BranchId, QueueError> {
        // NatsQueue doesn't create branches — RecordingQueue wraps this
        // and returns the real branch ID.
        Ok(vlinder_core::domain::BranchId::from(1))
    }
}

/// Serialize a routing key to a NATS subject string (ADR 096 §8).
///
/// This is the single point of truth for `RoutingKey` → NATS subject
/// serialization. Injectivity: distinct routing keys produce distinct
/// subjects (verified by tests in `routing_key.rs`).
fn routing_key_to_subject(key: &RoutingKey) -> String {
    let prefix = format!("vlinder.{}.{}.{}", key.session, key.branch, key.submission);
    let suffix = match &key.kind {
        RoutingKind::Invoke {
            harness,
            runtime,
            agent,
        } => {
            format!("invoke.{harness}.{runtime}.{agent}")
        }
        RoutingKind::Complete { agent, harness } => {
            format!("complete.{agent}.{harness}")
        }
        RoutingKind::Request {
            agent,
            service,
            operation,
            sequence,
        } => {
            format!(
                "req.{}.{}.{}.{}.{}",
                agent,
                service.service_type(),
                service.backend_str(),
                operation,
                sequence,
            )
        }
        RoutingKind::Response {
            service,
            agent,
            operation,
            sequence,
        } => {
            format!(
                "res.{}.{}.{}.{}.{}",
                service.service_type(),
                service.backend_str(),
                agent,
                operation,
                sequence,
            )
        }
        RoutingKind::Delegate { caller, target } => {
            format!("delegate.{caller}.{target}")
        }
        RoutingKind::DelegateReply {
            caller,
            target,
            nonce,
        } => {
            format!("delegate-reply.{caller}.{target}.{nonce}")
        }
        RoutingKind::Repair { harness, agent } => {
            format!("repair.{harness}.{agent}")
        }
        RoutingKind::Fork { agent_name } => {
            format!("fork.{agent_name}")
        }
        RoutingKind::Promote { agent_name } => {
            format!("promote.{agent_name}")
        }
    };
    format!("{prefix}.{suffix}")
}

/// Parse a NATS subject back into a `RoutingKey`.
///
/// Inverse of `routing_key_to_subject`. Returns `None` for subjects
/// that don't match the `vlinder.{session}.{branch}.{submission}.{type}...` format.
pub fn subject_to_routing_key(subject: &str) -> Option<RoutingKey> {
    let s: Vec<&str> = subject.split('.').collect();
    if s.len() < 5 || s[0] != "vlinder" {
        return None;
    }

    let session = SessionId::try_from(s[1].to_string()).ok()?;
    let branch = BranchId::from(s[2].parse::<i64>().unwrap_or(0));
    let submission = SubmissionId::from(s[3].to_string());

    let kind = match s[4] {
        "invoke" if s.len() == 8 => Some(RoutingKind::Invoke {
            harness: HarnessType::from_str(s[5]).ok()?,
            runtime: RuntimeType::from_str(s[6]).ok()?,
            agent: AgentId::new(s[7]),
        }),
        "req" if s.len() == 10 => Some(RoutingKind::Request {
            agent: AgentId::new(s[5]),
            service: ServiceBackend::from_parts(ServiceType::from_str(s[6]).ok()?, s[7])?,
            operation: Operation::from_str(s[8]).ok()?,
            sequence: Sequence::from(s[9].parse::<u32>().ok()?),
        }),
        "res" if s.len() == 10 => Some(RoutingKind::Response {
            service: ServiceBackend::from_parts(ServiceType::from_str(s[5]).ok()?, s[6])?,
            agent: AgentId::new(s[7]),
            operation: Operation::from_str(s[8]).ok()?,
            sequence: Sequence::from(s[9].parse::<u32>().ok()?),
        }),
        "complete" if s.len() == 7 => Some(RoutingKind::Complete {
            agent: AgentId::new(s[5]),
            harness: HarnessType::from_str(s[6]).ok()?,
        }),
        "delegate" if s.len() == 7 => Some(RoutingKind::Delegate {
            caller: AgentId::new(s[5]),
            target: AgentId::new(s[6]),
        }),
        "delegate-reply" if s.len() == 8 => Some(RoutingKind::DelegateReply {
            caller: AgentId::new(s[5]),
            target: AgentId::new(s[6]),
            nonce: Nonce::new(s[7]),
        }),
        "repair" if s.len() == 7 => Some(RoutingKind::Repair {
            harness: HarnessType::from_str(s[5]).ok()?,
            agent: AgentId::new(s[6]),
        }),
        "fork" if s.len() == 6 => Some(RoutingKind::Fork {
            agent_name: s[5].to_string(),
        }),
        "promote" if s.len() == 6 => Some(RoutingKind::Promote {
            agent_name: s[5].to_string(),
        }),
        _ => None,
    };

    kind.map(|kind| RoutingKey {
        session,
        branch,
        submission,
        kind,
    })
}

/// Serialize an `InvokeMessage` into NATS headers (sans payload).
///
/// Inverse of `from_nats_headers` for the Invoke variant.
/// This is the single source of truth for which fields go into NATS headers;
/// `send_invoke` delegates here.
pub fn invoke_to_nats_headers(msg: &InvokeMessage) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("msg-id".to_string(), msg.id.as_str().to_string());
    h.insert("protocol-version".to_string(), msg.protocol_version.clone());
    h.insert("session-id".to_string(), msg.session.as_str().to_string());
    if let Some(ref state) = msg.state {
        h.insert("state".to_string(), state.clone());
    }
    if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
        h.insert("diagnostics".to_string(), diag_json);
    }
    if !msg.dag_parent.is_empty() {
        h.insert("dag-parent".to_string(), msg.dag_parent.to_string());
    }
    h
}

/// Serialize a `RequestMessage` into NATS headers (sans payload).
pub fn request_to_nats_headers(msg: &RequestMessage) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("msg-id".to_string(), msg.id.as_str().to_string());
    h.insert("protocol-version".to_string(), msg.protocol_version.clone());
    h.insert("session-id".to_string(), msg.session.as_str().to_string());
    if let Some(ref state) = msg.state {
        h.insert("state".to_string(), state.clone());
    }
    if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
        h.insert("diagnostics".to_string(), diag_json);
    }
    if let Some(ref checkpoint) = msg.checkpoint {
        h.insert("checkpoint".to_string(), checkpoint.clone());
    }
    h
}

/// Serialize a `ResponseMessage` into NATS headers (sans payload).
pub fn response_to_nats_headers(msg: &ResponseMessage) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("msg-id".to_string(), msg.id.as_str().to_string());
    h.insert("protocol-version".to_string(), msg.protocol_version.clone());
    h.insert("session-id".to_string(), msg.session.as_str().to_string());
    h.insert(
        "correlation-id".to_string(),
        msg.correlation_id.as_str().to_string(),
    );
    h.insert("status-code".to_string(), msg.status_code.to_string());
    if let Some(ref state) = msg.state {
        h.insert("state".to_string(), state.clone());
    }
    if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
        h.insert("diagnostics".to_string(), diag_json);
    }
    h
}

/// Serialize a `CompleteMessage` into NATS headers (sans payload).
pub fn complete_to_nats_headers(msg: &CompleteMessage) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("msg-id".to_string(), msg.id.as_str().to_string());
    h.insert("protocol-version".to_string(), msg.protocol_version.clone());
    h.insert("session-id".to_string(), msg.session.as_str().to_string());
    if let Some(ref state) = msg.state {
        h.insert("state".to_string(), state.clone());
    }
    if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
        h.insert("diagnostics".to_string(), diag_json);
    }
    h
}

/// Serialize a `DelegateMessage` into NATS headers (sans payload).
pub fn delegate_to_nats_headers(msg: &DelegateMessage) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("msg-id".to_string(), msg.id.as_str().to_string());
    h.insert("protocol-version".to_string(), msg.protocol_version.clone());
    h.insert("session-id".to_string(), msg.session.as_str().to_string());
    h.insert("nonce".to_string(), msg.nonce.as_str().to_string());
    if let Some(ref state) = msg.state {
        h.insert("state".to_string(), state.clone());
    }
    if let Ok(diag_json) = serde_json::to_string(&msg.diagnostics) {
        h.insert("diagnostics".to_string(), diag_json);
    }
    h
}

/// Serialize a `RepairMessage` into NATS headers (sans payload).
#[allow(dead_code)]
pub fn repair_to_nats_headers(msg: &RepairMessage) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("msg-id".to_string(), msg.id.as_str().to_string());
    h.insert("protocol-version".to_string(), msg.protocol_version.clone());
    h.insert("session-id".to_string(), msg.session.as_str().to_string());
    h.insert("dag-parent".to_string(), msg.dag_parent.to_string());
    h.insert("checkpoint".to_string(), msg.checkpoint.clone());
    h.insert(
        "service".to_string(),
        msg.service.service_type().as_str().to_string(),
    );
    h.insert("backend".to_string(), msg.service.backend_str().to_string());
    h.insert("operation".to_string(), msg.operation.as_str().to_string());
    h.insert("sequence".to_string(), msg.sequence.to_string());
    if let Some(ref state) = msg.state {
        h.insert("state".to_string(), state.clone());
    }
    h
}

/// Reconstruct typed message headers from a `RoutingKey` and NATS header map.
///
/// The `RoutingKey` provides the message type discriminant and routing fields
/// (agent, harness, service, etc.) — already parsed by `subject_to_routing_key`.
/// The header map provides session-scoped fields (msg-id, session-id, state,
/// diagnostics) that aren't part of routing.
pub fn from_nats_headers<S: BuildHasher>(
    key: &RoutingKey,
    headers: &HashMap<String, String, S>,
) -> Option<vlinder_core::domain::ObservableMessageHeaders> {
    use vlinder_core::domain::{MessageDetails, ObservableMessageHeaders};

    let id = MessageId::from(headers.get("msg-id")?.clone());
    let protocol_version = headers.get("protocol-version").cloned().unwrap_or_default();
    let state = headers.get("state").cloned();

    let details = match &key.kind {
        RoutingKind::Invoke { .. } => {
            let diagnostics = headers
                .get("diagnostics")
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| InvokeDiagnostics {
                    harness_version: String::new(),
                });
            let dag_parent =
                DagNodeId::from(headers.get("dag-parent").cloned().unwrap_or_default());
            Some(MessageDetails::Invoke {
                diagnostics,
                dag_parent,
            })
        }
        RoutingKind::Request { .. } => {
            let diagnostics = headers
                .get("diagnostics")
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| RequestDiagnostics {
                    sequence: 0,
                    endpoint: String::new(),
                    request_bytes: 0,
                    received_at_ms: 0,
                });
            Some(MessageDetails::Request {
                diagnostics,
                checkpoint: headers.get("checkpoint").cloned(),
            })
        }
        RoutingKind::Response { .. } => {
            let diagnostics = headers
                .get("diagnostics")
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(ServiceDiagnostics::placeholder);
            let correlation_id = MessageId::from(headers.get("correlation-id")?.clone());
            let status_code = headers
                .get("status-code")
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(200);
            Some(MessageDetails::Response {
                diagnostics,
                correlation_id,
                status_code,
                checkpoint: headers.get("checkpoint").cloned(),
            })
        }
        RoutingKind::Complete { .. } => {
            let diagnostics = headers
                .get("diagnostics")
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| RuntimeDiagnostics::placeholder(0));
            Some(MessageDetails::Complete { diagnostics })
        }
        RoutingKind::Delegate { .. } => {
            let diagnostics = headers
                .get("diagnostics")
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| DelegateDiagnostics {
                    runtime: RuntimeDiagnostics::placeholder(0),
                });
            let nonce = Nonce::new(headers.get("nonce")?.clone());
            Some(MessageDetails::Delegate { diagnostics, nonce })
        }
        RoutingKind::DelegateReply { .. } => None,
        RoutingKind::Repair { .. } => {
            let dag_parent = DagNodeId::from(headers.get("dag-parent").cloned()?);
            let checkpoint = headers.get("checkpoint").cloned()?;
            let service = ServiceBackend::from_parts(
                ServiceType::from_str(headers.get("service")?).ok()?,
                headers.get("backend")?,
            )?;
            let operation = Operation::from_str(headers.get("operation")?).ok()?;
            let sequence = Sequence::from(headers.get("sequence")?.parse::<u32>().ok()?);
            Some(MessageDetails::Repair {
                dag_parent,
                checkpoint,
                service,
                operation,
                sequence,
            })
        }
        RoutingKind::Fork { .. } => {
            let branch_name = headers.get("branch-name").cloned()?;
            let fork_point = DagNodeId::from(headers.get("fork-point").cloned()?);
            Some(MessageDetails::Fork {
                branch_name,
                fork_point,
            })
        }
        RoutingKind::Promote { .. } => Some(MessageDetails::Promote),
    };

    details.map(|details| ObservableMessageHeaders {
        id,
        protocol_version,
        state,
        routing_key: key.clone(),
        details,
    })
}

/// Derive a stable consumer name from a filter pattern.
///
/// NATS consumer names must be alphanumeric + dash/underscore.
/// We replace dots and wildcards with underscores.
fn filter_to_consumer_name(filter: &str) -> String {
    filter.replace('.', "_").replace('*', "W").replace('>', "G")
}

/// Extract a header value from NATS headers.
fn get_header(headers: &async_nats::HeaderMap, key: &str) -> Result<String, QueueError> {
    headers
        .get(key)
        .map(std::string::ToString::to_string)
        .ok_or_else(|| QueueError::ReceiveFailed(format!("missing header: {key}")))
}

// ============================================================================
// Tests — subject serialization injectivity (ADR 096 §9)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use vlinder_core::domain::{InferenceBackendType, ObjectStorageType, VectorStorageType};

    fn session() -> SessionId {
        SessionId::try_from("00000000-0000-4000-8000-000000000001".to_string()).unwrap()
    }
    fn timeline() -> BranchId {
        BranchId::from(1)
    }
    fn timeline_alt() -> BranchId {
        BranchId::from(2)
    }
    fn submission() -> SubmissionId {
        SubmissionId::from("sub-1".to_string())
    }
    fn submission_alt() -> SubmissionId {
        SubmissionId::from("sub-2".to_string())
    }
    fn agent() -> AgentId {
        AgentId::new("echo")
    }
    fn agent_alt() -> AgentId {
        AgentId::new("pensieve")
    }

    // ========================================================================
    // Format sanity — subjects have the expected shape
    // ========================================================================

    #[test]
    fn invoke_subject_format() {
        let key = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!(
                "vlinder.{}.{}.{}.invoke.cli.container.echo",
                session(),
                timeline(),
                submission()
            ),
        );
    }

    #[test]
    fn request_subject_format() {
        let key = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!(
                "vlinder.{}.{}.{}.req.echo.kv.sqlite.get.1",
                session(),
                timeline(),
                submission()
            ),
        );
    }

    #[test]
    fn response_subject_format() {
        let key = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Response {
                service: ServiceBackend::Infer(InferenceBackendType::Ollama),
                agent: agent(),
                operation: Operation::Run,
                sequence: Sequence::from(3),
            },
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!(
                "vlinder.{}.{}.{}.res.infer.ollama.echo.run.3",
                session(),
                timeline(),
                submission()
            ),
        );
    }

    #[test]
    fn complete_subject_format() {
        let key = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Complete {
                agent: agent(),
                harness: HarnessType::Web,
            },
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!(
                "vlinder.{}.{}.{}.complete.echo.web",
                session(),
                timeline(),
                submission()
            ),
        );
    }

    #[test]
    fn delegate_subject_format() {
        let key = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Delegate {
                caller: agent(),
                target: agent_alt(),
            },
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!(
                "vlinder.{}.{}.{}.delegate.echo.pensieve",
                session(),
                timeline(),
                submission()
            ),
        );
    }

    #[test]
    fn delegate_reply_subject_format() {
        let key = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::DelegateReply {
                caller: agent(),
                target: agent_alt(),
                nonce: Nonce::new("abc123"),
            },
        };
        assert_eq!(
            routing_key_to_subject(&key),
            format!(
                "vlinder.{}.{}.{}.delegate-reply.echo.pensieve.abc123",
                session(),
                timeline(),
                submission()
            ),
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
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline_alt(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn invoke_injective_by_submission() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission_alt(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn invoke_injective_by_harness() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Web,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn invoke_injective_by_agent() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent_alt(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn request_injective_by_service() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Vec(VectorStorageType::SqliteVec),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn request_injective_by_backend() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::InMemory),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn request_injective_by_operation() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Put,
                sequence: Sequence::first(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn request_injective_by_sequence() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::from(2),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn response_injective_by_agent() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Response {
                service: ServiceBackend::Infer(InferenceBackendType::Ollama),
                agent: agent(),
                operation: Operation::Run,
                sequence: Sequence::first(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Response {
                service: ServiceBackend::Infer(InferenceBackendType::Ollama),
                agent: agent_alt(),
                operation: Operation::Run,
                sequence: Sequence::first(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn response_injective_by_backend() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Response {
                service: ServiceBackend::Infer(InferenceBackendType::Ollama),
                agent: agent(),
                operation: Operation::Run,
                sequence: Sequence::first(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Response {
                service: ServiceBackend::Infer(InferenceBackendType::OpenRouter),
                agent: agent(),
                operation: Operation::Run,
                sequence: Sequence::first(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn complete_injective_by_harness() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Complete {
                agent: agent(),
                harness: HarnessType::Cli,
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Complete {
                agent: agent(),
                harness: HarnessType::Web,
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn complete_injective_by_agent() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Complete {
                agent: agent(),
                harness: HarnessType::Cli,
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Complete {
                agent: agent_alt(),
                harness: HarnessType::Cli,
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn delegate_injective_by_caller() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Delegate {
                caller: agent(),
                target: agent_alt(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Delegate {
                caller: agent_alt(),
                target: agent_alt(),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn delegate_injective_by_target() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Delegate {
                caller: agent(),
                target: agent_alt(),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Delegate {
                caller: agent(),
                target: AgentId::new("fact-checker"),
            },
        };
        assert_injective(&a, &b);
    }

    #[test]
    fn delegate_reply_injective_by_nonce() {
        let a = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::DelegateReply {
                caller: agent(),
                target: agent_alt(),
                nonce: Nonce::new("nonce-1"),
            },
        };
        let b = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::DelegateReply {
                caller: agent(),
                target: agent_alt(),
                nonce: Nonce::new("nonce-2"),
            },
        };
        assert_injective(&a, &b);
    }

    // ========================================================================
    // Cross-variant injectivity — different message types never collide
    // ========================================================================

    #[test]
    fn invoke_and_complete_subjects_differ() {
        let invoke = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        };
        let complete = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Complete {
                agent: agent(),
                harness: HarnessType::Cli,
            },
        };
        assert_injective(&invoke, &complete);
    }

    #[test]
    fn request_and_response_subjects_differ() {
        let request = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        let response = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Response {
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                agent: agent(),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        };
        assert_injective(&request, &response);
    }

    #[test]
    fn delegate_and_delegate_reply_subjects_differ() {
        let delegate = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Delegate {
                caller: agent(),
                target: agent_alt(),
            },
        };
        let reply = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::DelegateReply {
                caller: agent(),
                target: agent_alt(),
                nonce: Nonce::new("n"),
            },
        };
        assert_injective(&delegate, &reply);
    }

    // ========================================================================
    // Round-trip: routing_key_to_subject → subject_to_routing_key
    // ========================================================================

    fn assert_round_trips(key: &RoutingKey) {
        let subject = routing_key_to_subject(key);
        let recovered = subject_to_routing_key(&subject)
            .unwrap_or_else(|| panic!("failed to parse subject: {subject}"));
        assert_eq!(&recovered, key, "round-trip failed for subject: {subject}");
    }

    #[test]
    fn invoke_round_trips() {
        assert_round_trips(&RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Invoke {
                harness: HarnessType::Cli,
                runtime: RuntimeType::Container,
                agent: agent(),
            },
        });
    }

    #[test]
    fn request_round_trips() {
        assert_round_trips(&RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Request {
                agent: agent(),
                service: ServiceBackend::Kv(ObjectStorageType::Sqlite),
                operation: Operation::Get,
                sequence: Sequence::first(),
            },
        });
    }

    #[test]
    fn response_round_trips() {
        assert_round_trips(&RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Response {
                service: ServiceBackend::Infer(InferenceBackendType::Ollama),
                agent: agent(),
                operation: Operation::Run,
                sequence: Sequence::from(3),
            },
        });
    }

    #[test]
    fn complete_round_trips() {
        assert_round_trips(&RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Complete {
                agent: agent(),
                harness: HarnessType::Grpc,
            },
        });
    }

    #[test]
    fn delegate_round_trips() {
        assert_round_trips(&RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::Delegate {
                caller: agent(),
                target: agent_alt(),
            },
        });
    }

    #[test]
    fn delegate_reply_round_trips() {
        assert_round_trips(&RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::DelegateReply {
                caller: agent(),
                target: agent_alt(),
                nonce: Nonce::new("abc123"),
            },
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

    // ========================================================================
    // from_nats_headers: RoutingKey + headers → ObservableMessageHeaders
    // ========================================================================

    /// Build a test `InvokeMessage` for header round-trip tests.
    fn test_invoke_message(state: Option<String>) -> InvokeMessage {
        InvokeMessage::new(
            timeline(),
            submission(),
            SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap(),
            HarnessType::Cli,
            RuntimeType::Container,
            agent(),
            b"hello-payload".to_vec(),
            state,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        )
    }

    #[test]
    fn invoke_headers_round_trip() {
        use vlinder_core::domain::ObservableMessage;

        let original = test_invoke_message(Some("state-abc".to_string()));
        let key = original.routing_key();
        let headers = invoke_to_nats_headers(&original);

        let recovered = from_nats_headers(&key, &headers)
            .expect("should produce Invoke headers")
            .assemble(original.payload.clone());

        if let ObservableMessage::Invoke(m) = &recovered {
            assert_eq!(m.id, original.id);
            assert_eq!(m.protocol_version, original.protocol_version);
            assert_eq!(m.branch, original.branch);
            assert_eq!(m.submission, original.submission);
            assert_eq!(m.session, original.session);
            assert_eq!(m.harness, original.harness);
            assert_eq!(m.runtime, original.runtime);
            assert_eq!(m.agent_id, original.agent_id);
            assert_eq!(m.payload, original.payload);
            assert_eq!(m.state, original.state);
        } else {
            panic!("expected Invoke, got {recovered:?}");
        }
    }

    #[test]
    fn invoke_headers_round_trip_without_state() {
        use vlinder_core::domain::ObservableMessage;

        let original = test_invoke_message(None);
        let key = original.routing_key();
        let headers = invoke_to_nats_headers(&original);

        let recovered = from_nats_headers(&key, &headers)
            .unwrap()
            .assemble(original.payload.clone());

        if let ObservableMessage::Invoke(m) = &recovered {
            assert_eq!(m.state, None);
            assert_eq!(m.id, original.id);
        } else {
            panic!("expected Invoke");
        }
    }

    #[test]
    fn from_nats_headers_invoke_missing_msg_id_returns_none() {
        let original = test_invoke_message(None);
        let key = original.routing_key();
        let mut headers = invoke_to_nats_headers(&original);
        headers.remove("msg-id");

        assert!(from_nats_headers(&key, &headers).is_none());
    }

    #[test]
    fn from_nats_headers_delegate_reply_returns_none() {
        let key = RoutingKey {
            session: session(),
            branch: timeline(),
            submission: submission(),
            kind: RoutingKind::DelegateReply {
                caller: agent(),
                target: agent_alt(),
                nonce: Nonce::new("n"),
            },
        };
        assert!(from_nats_headers(&key, &HashMap::new()).is_none());
    }

    // ========================================================================
    // Request header round-trip
    // ========================================================================

    #[test]
    fn request_headers_round_trip() {
        use vlinder_core::domain::ObservableMessage;

        let original = RequestMessage::new(
            timeline(),
            submission(),
            SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap(),
            agent(),
            ServiceBackend::Kv(ObjectStorageType::Sqlite),
            Operation::Get,
            Sequence::first(),
            b"key-data".to_vec(),
            Some("state-x".to_string()),
            RequestDiagnostics {
                sequence: 1,
                endpoint: "/test".to_string(),
                request_bytes: 8,
                received_at_ms: 0,
            },
        );
        let key = original.routing_key();
        let headers = request_to_nats_headers(&original);

        let recovered = from_nats_headers(&key, &headers)
            .expect("should produce Request headers")
            .assemble(original.payload.clone());

        if let ObservableMessage::Request(m) = &recovered {
            assert_eq!(m.id, original.id);
            assert_eq!(m.submission, original.submission);
            assert_eq!(m.session, original.session);
            assert_eq!(m.agent_id, original.agent_id);
            assert_eq!(m.service, original.service);
            assert_eq!(m.operation, original.operation);
            assert_eq!(m.sequence, original.sequence);
            assert_eq!(m.payload, original.payload);
            assert_eq!(m.state, original.state);
        } else {
            panic!("expected Request, got {recovered:?}");
        }
    }

    // ========================================================================
    // Response header round-trip
    // ========================================================================

    #[test]
    fn response_headers_round_trip() {
        use vlinder_core::domain::ObservableMessage;

        let request = RequestMessage::new(
            timeline(),
            submission(),
            SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap(),
            agent(),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
            Operation::Run,
            Sequence::from(3),
            b"prompt".to_vec(),
            None,
            RequestDiagnostics {
                sequence: 3,
                endpoint: "/infer".to_string(),
                request_bytes: 6,
                received_at_ms: 0,
            },
        );
        let original = ResponseMessage::from_request(&request, b"reply-data".to_vec());
        let key = original.routing_key();
        let headers = response_to_nats_headers(&original);

        let recovered = from_nats_headers(&key, &headers)
            .expect("should produce Response headers")
            .assemble(original.payload.clone());

        if let ObservableMessage::Response(m) = &recovered {
            assert_eq!(m.id, original.id);
            assert_eq!(m.submission, original.submission);
            assert_eq!(m.session, original.session);
            assert_eq!(m.agent_id, original.agent_id);
            assert_eq!(m.service, original.service);
            assert_eq!(m.operation, original.operation);
            assert_eq!(m.sequence, original.sequence);
            assert_eq!(m.correlation_id, original.correlation_id);
            assert_eq!(m.status_code, original.status_code);
            assert_eq!(m.payload, original.payload);
        } else {
            panic!("expected Response, got {recovered:?}");
        }
    }

    // ========================================================================
    // Complete header round-trip
    // ========================================================================

    #[test]
    fn complete_headers_round_trip() {
        use vlinder_core::domain::ObservableMessage;

        let original = CompleteMessage::new(
            timeline(),
            submission(),
            SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap(),
            agent(),
            HarnessType::Grpc,
            b"done".to_vec(),
            Some("state-z".to_string()),
            RuntimeDiagnostics::placeholder(0),
        );
        let key = original.routing_key();
        let headers = complete_to_nats_headers(&original);

        let recovered = from_nats_headers(&key, &headers)
            .expect("should produce Complete headers")
            .assemble(original.payload.clone());

        if let ObservableMessage::Complete(m) = &recovered {
            assert_eq!(m.id, original.id);
            assert_eq!(m.submission, original.submission);
            assert_eq!(m.session, original.session);
            assert_eq!(m.agent_id, original.agent_id);
            assert_eq!(m.harness, original.harness);
            assert_eq!(m.payload, original.payload);
            assert_eq!(m.state, original.state);
        } else {
            panic!("expected Complete, got {recovered:?}");
        }
    }

    // ========================================================================
    // Delegate header round-trip
    // ========================================================================

    #[test]
    fn delegate_headers_round_trip() {
        use vlinder_core::domain::ObservableMessage;

        let original = DelegateMessage::new(
            timeline(),
            submission(),
            SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap(),
            agent(),
            agent_alt(),
            b"task-data".to_vec(),
            Nonce::new("nonce-42"),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );
        let key = original.routing_key();
        let headers = delegate_to_nats_headers(&original);

        let recovered = from_nats_headers(&key, &headers)
            .expect("should produce Delegate headers")
            .assemble(original.payload.clone());

        if let ObservableMessage::Delegate(m) = &recovered {
            assert_eq!(m.id, original.id);
            assert_eq!(m.submission, original.submission);
            assert_eq!(m.session, original.session);
            assert_eq!(m.caller, original.caller);
            assert_eq!(m.target, original.target);
            assert_eq!(m.nonce, original.nonce);
            assert_eq!(m.payload, original.payload);
            assert_eq!(m.state, original.state);
        } else {
            panic!("expected Delegate, got {recovered:?}");
        }
    }
}
