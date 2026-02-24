//! QueueBridge — queue-backed agent SDK (ADR 074, 076).
//!
//! Routes typed platform service calls through the MessageQueue.
//! KV and vector operations have moved to provider hostnames;
//! the bridge now handles delegation and invoke context only.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use vlinder_core::domain::{
    AgentId, Registry, RoutingKey,
    DelegateMessage, DelegateDiagnostics, ContainerDiagnostics,
    InvokeMessage, MessageQueue, Nonce,
    SequenceCounter,
};

/// Routes agent SDK calls to the appropriate backend service.
///
/// Constructed once per container, shared across all service calls.
/// The invoke context is updated per invocation via `update_invoke()`.
pub struct QueueBridge {
    pub queue: Arc<dyn MessageQueue + Send + Sync>,
    pub registry: Arc<dyn Registry>,
    /// The invoke that triggered this execution — carries submission + agent_id.
    /// Updated per invocation so SDK calls route on the correct submission ID.
    pub invoke: RwLock<InvokeMessage>,
    /// Sequence counter — incremented per service call, reset per invocation
    pub sequence: SequenceCounter,
    /// Maps delegation handles (nonce strings) to reply routing keys (ADR 096 §7).
    /// Populated by delegate(), consumed by wait().
    #[allow(dead_code)]
    pub pending_replies: RwLock<HashMap<String, RoutingKey>>,
}

impl QueueBridge {
    /// Update the invoke context for a new invocation and reset the sequence counter.
    pub fn update_invoke(&self, invoke: InvokeMessage) {
        *self.invoke.write().unwrap() = invoke;
        self.sequence.reset();
    }

    // ========================================================================
    // SDK operations
    // ========================================================================

    #[allow(dead_code)]
    pub fn delegate(&self, target_agent: &str, input: &str) -> Result<String, String> {
        let _agent = self.registry.get_agent_by_name(target_agent)
            .ok_or_else(|| format!("delegate: target agent '{}' not found", target_agent))?;

        let invoke = self.invoke.read().unwrap();
        let caller = invoke.agent_id.clone();
        let target = AgentId::new(target_agent);
        let sha = invoke.submission.to_string();
        let nonce = Nonce::generate();

        let delegate = DelegateMessage::new(
            invoke.timeline.clone(),
            invoke.submission.clone(),
            invoke.session.clone(),
            caller.clone(),
            target.clone(),
            input.as_bytes().to_vec(),
            nonce.clone(),
            None,
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );
        let reply_key = delegate.reply_routing_key();
        let handle = nonce.as_str().to_string();
        drop(invoke);

        tracing::info!(
            sha = %sha, event = "delegation.sent",
            caller = %caller, target = %target,
            nonce = %nonce, "Delegating to agent"
        );

        // Store reply routing key for wait()
        self.pending_replies.write().unwrap().insert(handle.clone(), reply_key);

        self.queue.send_delegate(delegate)
            .map_err(|e| format!("delegate send error: {}", e))?;

        Ok(handle)
    }

    #[allow(dead_code)]
    pub fn wait(&self, handle: &str) -> Result<Vec<u8>, String> {
        let sha = self.invoke.read().unwrap().submission.to_string();
        let reply_key = self.pending_replies.read().unwrap().get(handle).cloned()
            .ok_or_else(|| format!("wait: unknown delegation handle '{}'", handle))?;

        tracing::debug!(sha = %sha, handle = %handle, "wait: polling for delegation result");
        let poll_start = std::time::Instant::now();
        let mut poll_count: u64 = 0;

        loop {
            match self.queue.receive_delegate_reply(&reply_key) {
                Ok((complete, ack)) => {
                    let payload = complete.payload.clone();
                    let _ = ack();
                    self.pending_replies.write().unwrap().remove(handle);
                    tracing::info!(
                        sha = %sha, event = "delegation.completed",
                        handle = %handle, polls = poll_count,
                        elapsed = ?poll_start.elapsed(),
                        "Delegation result received"
                    );
                    return Ok(payload);
                }
                Err(_) => {
                    poll_count += 1;
                    if poll_count % 100 == 0 {
                        tracing::warn!(
                            sha = %sha,
                            handle = %handle, polls = poll_count,
                            elapsed = ?poll_start.elapsed(),
                            "wait: still waiting for delegation result"
                        );
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vlinder_core::queue::InMemoryQueue;
    use vlinder_core::domain::{
        HarnessType, InvokeDiagnostics, RuntimeType, SessionId, SubmissionId, TimelineId,
        SecretStore, InMemoryRegistry, InMemorySecretStore,
    };

    fn test_secret_store() -> Arc<dyn SecretStore> {
        Arc::new(InMemorySecretStore::new())
    }

    /// Build a QueueBridge wired to in-memory backends for unit testing.
    fn test_bridge() -> QueueBridge {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry: Arc<dyn Registry> = Arc::new(InMemoryRegistry::new(test_secret_store()));
        let invoke = InvokeMessage::new(
            TimelineId::main(),
            SubmissionId::new(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("echo"),
            b"hello".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );
        QueueBridge {
            queue,
            registry,
            invoke: RwLock::new(invoke),
            sequence: SequenceCounter::new(),
            pending_replies: RwLock::new(HashMap::new()),
        }
    }

    #[test]
    fn update_invoke_resets_sequence() {
        let bridge = test_bridge();

        let invoke = InvokeMessage::new(
            TimelineId::main(),
            SubmissionId::new(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("echo"),
            b"new input".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );

        bridge.update_invoke(invoke);
        // Sequence counter was reset — first call should return 1
        let seq = bridge.sequence.next();
        assert_eq!(seq.as_u32(), 1);
    }
}
