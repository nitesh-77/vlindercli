//! NATS-backed message queue with JetStream durability (ADR 044).
//!
//! Provides a sync facade over the async NATS client. The runtime is owned
//! internally, so callers use simple blocking APIs while getting async I/O.
//!
//! Uses typed messages exclusively for full observability.

use std::sync::Arc;

use async_nats::jetstream::{self, stream};
use tokio::runtime::Runtime;

use super::{
    CompleteMessage, InvokeMessage, MessageQueue, QueueError,
    RequestMessage, ResponseMessage,
};
use crate::domain::ResourceId;

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

    fn receive_invoke(&self, _subject_pattern: &str) -> Result<(InvokeMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // TODO: Implement NATS typed receive with header extraction
        Err(QueueError::ReceiveFailed("typed receive not yet implemented for NATS".to_string()))
    }

    fn receive_request(&self, _service: &str, _backend: &str, _operation: &str) -> Result<(RequestMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // TODO: Implement NATS typed receive with header extraction
        Err(QueueError::ReceiveFailed("typed receive not yet implemented for NATS".to_string()))
    }

    fn receive_response(&self, _subject_pattern: &str) -> Result<(ResponseMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // TODO: Implement NATS typed receive with header extraction
        Err(QueueError::ReceiveFailed("typed receive not yet implemented for NATS".to_string()))
    }

    fn receive_complete(&self, _harness_pattern: &str) -> Result<(CompleteMessage, Box<dyn FnOnce() -> Result<(), QueueError> + Send>), QueueError> {
        // TODO: Implement NATS typed receive with header extraction
        Err(QueueError::ReceiveFailed("typed receive not yet implemented for NATS".to_string()))
    }
}

/// Extract a short name from a ResourceId for NATS subjects.
fn agent_short_name(agent_id: &ResourceId) -> String {
    if let Some(path) = agent_id.path() {
        if let Some(filename) = path.rsplit('/').next() {
            let name = filename.strip_suffix(".wasm").unwrap_or(filename);
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    if let Some(authority) = agent_id.authority() {
        return authority.to_string();
    }
    agent_id.as_str().to_string()
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
