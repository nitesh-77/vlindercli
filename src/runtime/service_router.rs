//! ServiceRouter — shared agent→service call handler.
//!
//! Used by ContainerRuntime (via HTTP bridge).
//! Contains the core logic: parse SdkMessage, resolve hop, build RequestMessage,
//! send to queue, poll for ResponseMessage, return payload.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::domain::{ObjectStorageType, SdkMessage, VectorStorageType};
use crate::queue::{InvokeMessage, MessageQueue, RequestMessage, SequenceCounter};

/// Routes agent SDK calls to the appropriate backend service.
///
/// Constructed once per container, shared across all service calls.
/// The invoke context is updated per invocation via `update_invoke()`.
pub(crate) struct ServiceRouter {
    pub(crate) queue: Arc<dyn MessageQueue + Send + Sync>,
    /// The invoke that triggered this execution — carries submission + agent_id.
    /// Updated per invocation so SDK calls route on the correct submission ID.
    pub(crate) invoke: RwLock<InvokeMessage>,
    /// Resolved backends from agent config (None if agent didn't declare storage)
    pub(crate) kv_backend: Option<ObjectStorageType>,
    pub(crate) vec_backend: Option<VectorStorageType>,
    /// Model name → backend string mapping (built from agent's declared models)
    pub(crate) model_backends: HashMap<String, String>,
    /// Sequence counter — incremented per service call, reset per invocation
    pub(crate) sequence: SequenceCounter,
}

impl ServiceRouter {
    /// Update the invoke context for a new invocation and reset the sequence counter.
    pub(crate) fn update_invoke(&self, invoke: InvokeMessage) {
        *self.invoke.write().unwrap() = invoke;
        self.sequence.reset();
    }

    /// Dispatch a service call from the agent.
    ///
    /// Validates the payload, resolves the next hop, builds a typed request,
    /// sends it, and waits for the response. Returns the response payload.
    pub(crate) fn dispatch(&self, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let msg: SdkMessage = serde_json::from_slice(&payload)
            .map_err(|e| format!("invalid SDK message: {}", e))?;

        let hop = msg.hop(self.kv_backend, self.vec_backend, &self.model_backends)?;
        let seq = self.sequence.next();

        let invoke = self.invoke.read().unwrap();
        let request = RequestMessage::new(
            invoke.submission.clone(),
            invoke.agent_id.clone(),
            hop.service,
            hop.backend,
            hop.operation,
            seq,
            payload,
        );
        drop(invoke);

        self.queue.send_request(request.clone())
            .map_err(|e| format!("send error: {}", e))?;

        loop {
            if let Ok((response, ack)) = self.queue.receive_response(&request) {
                let payload = response.payload.clone();
                let _ = ack();
                return Ok(payload);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
