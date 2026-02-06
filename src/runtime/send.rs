//! SendFunctionData — shared agent→service call handler.
//!
//! Used by ContainerRuntime (via HTTP bridge).
//! Contains the core logic: parse SdkMessage, resolve hop, build RequestMessage,
//! send to queue, poll for ResponseMessage, return payload.

use std::sync::Arc;

use crate::domain::{ObjectStorageType, SdkMessage, VectorStorageType};
use crate::queue::{InvokeMessage, MessageQueue, RequestMessage, SequenceCounter};

/// Data needed to handle service calls on behalf of an agent.
///
/// Constructed once per agent invocation, shared across all service calls
/// during that invocation. The sequence counter tracks call ordering.
pub(crate) struct SendFunctionData {
    pub(crate) queue: Arc<dyn MessageQueue + Send + Sync>,
    /// The invoke that triggered this execution — carries submission + agent_id
    pub(crate) invoke: InvokeMessage,
    /// Resolved backends from agent config (None if agent didn't declare storage)
    pub(crate) kv_backend: Option<ObjectStorageType>,
    pub(crate) vec_backend: Option<VectorStorageType>,
    /// Sequence counter — incremented per service call
    pub(crate) sequence: SequenceCounter,
}

impl SendFunctionData {
    /// Handle a send call from the agent.
    ///
    /// Validates the payload, resolves the next hop, builds a typed request,
    /// sends it, and waits for the response. Returns the response payload.
    pub(crate) fn handle_send(&self, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let msg: SdkMessage = serde_json::from_slice(&payload)
            .map_err(|e| format!("invalid SDK message: {}", e))?;

        let hop = msg.hop(self.kv_backend, self.vec_backend)?;
        let seq = self.sequence.next();

        let request = RequestMessage::new(
            self.invoke.submission.clone(),
            self.invoke.agent_id.clone(),
            hop.service,
            hop.backend,
            hop.operation,
            seq,
            payload,
        );

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
