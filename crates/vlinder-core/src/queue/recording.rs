//! RecordingQueue — transactional outbox for synchronous DAG recording.
//!
//! Wraps any `MessageQueue` and records a `DagNode` into a `DagStore`
//! before forwarding each send. This eliminates the race condition where
//! a query sees stale state because the async NATS consumer hasn't
//! processed the latest messages yet.
//!
//! Receive and routing methods delegate straight through.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::workers::dag::build_dag_node;
use crate::domain::{
    Acknowledgement, CompleteMessage, DagStore, DelegateMessage, InvokeMessage, MessageQueue,
    ObservableMessage, QueueError, RequestMessage, ResponseMessage, SubmissionId,
};

/// A `MessageQueue` decorator that synchronously records DAG nodes on send.
///
/// Every `send_*` call: clone → convert to `ObservableMessage` → build
/// `DagNode` with Merkle chaining → insert into `DagStore` → forward
/// original message to inner queue.
///
/// DagStore write failures are logged but don't block message sending.
pub struct RecordingQueue {
    inner: Arc<dyn MessageQueue + Send + Sync>,
    store: Arc<dyn DagStore>,
    /// Per-session Merkle chain state: session_id → last node hash.
    chain: Mutex<HashMap<String, String>>,
}

impl RecordingQueue {
    pub fn new(inner: Arc<dyn MessageQueue + Send + Sync>, store: Arc<dyn DagStore>) -> Self {
        Self {
            inner,
            store,
            chain: Mutex::new(HashMap::new()),
        }
    }

    /// Record a DAG node for the given observable message, then update chain state.
    fn record(&self, observable: &ObservableMessage) {
        let session_id = observable.session().as_str().to_string();

        // Explicit dag_parent on Invoke overrides chain cache (fork/repair).
        let dag_parent_override = match observable {
            ObservableMessage::Invoke(m) if !m.dag_parent.is_empty() => Some(m.dag_parent.clone()),
            _ => None,
        };

        // Look up parent hash: dag_parent override, then in-memory cache, then DagStore fallback
        let parent_hash = dag_parent_override.unwrap_or_else(|| {
            {
                let chain = self.chain.lock().unwrap();
                chain.get(&session_id).cloned()
            }.unwrap_or_else(|| {
                self.store.latest_node_hash(&session_id)
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, session = %session_id, "Failed to read latest node hash");
                        None
                    })
                    .unwrap_or_default()
            })
        });

        let node = build_dag_node(observable, &parent_hash);
        let node_hash = node.hash.clone();

        if let Err(e) = self.store.insert_node(&node) {
            tracing::warn!(error = %e, hash = %node_hash, "Failed to record DAG node (outbox)");
        }

        // Update chain state regardless of store success — the hash is
        // deterministic, so even if the insert failed the next node should
        // chain from the correct parent.
        self.chain.lock().unwrap().insert(session_id, node_hash);
    }
}

impl MessageQueue for RecordingQueue {
    // -------------------------------------------------------------------------
    // Send methods — record DAG node, then forward
    // -------------------------------------------------------------------------

    fn send_invoke(&self, msg: InvokeMessage) -> Result<(), QueueError> {
        self.record(&msg.clone().into());
        self.inner.send_invoke(msg)
    }

    fn send_request(&self, msg: RequestMessage) -> Result<(), QueueError> {
        self.record(&msg.clone().into());
        self.inner.send_request(msg)
    }

    fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError> {
        self.record(&msg.clone().into());
        self.inner.send_response(msg)
    }

    fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError> {
        self.record(&msg.clone().into());
        self.inner.send_complete(msg)
    }

    fn send_delegate(&self, msg: DelegateMessage) -> Result<(), QueueError> {
        self.record(&msg.clone().into());
        self.inner.send_delegate(msg)
    }

    fn send_delegate_reply(
        &self,
        msg: CompleteMessage,
        reply_key: &crate::domain::RoutingKey,
    ) -> Result<(), QueueError> {
        self.record(&msg.clone().into());
        self.inner.send_delegate_reply(msg, reply_key)
    }

    // -------------------------------------------------------------------------
    // Receive methods — delegate straight through
    // -------------------------------------------------------------------------

    fn receive_invoke(
        &self,
        agent: &crate::domain::AgentId,
    ) -> Result<(InvokeMessage, Acknowledgement), QueueError> {
        self.inner.receive_invoke(agent)
    }

    fn receive_request(
        &self,
        service: crate::domain::ServiceBackend,
        operation: crate::domain::Operation,
    ) -> Result<(RequestMessage, Acknowledgement), QueueError> {
        self.inner.receive_request(service, operation)
    }

    fn receive_response(
        &self,
        request: &RequestMessage,
    ) -> Result<(ResponseMessage, Acknowledgement), QueueError> {
        self.inner.receive_response(request)
    }

    fn receive_complete(
        &self,
        submission: &SubmissionId,
        harness: crate::domain::HarnessType,
    ) -> Result<(CompleteMessage, Acknowledgement), QueueError> {
        self.inner.receive_complete(submission, harness)
    }

    // -------------------------------------------------------------------------
    // Delegation methods — delegate straight through
    // -------------------------------------------------------------------------

    fn receive_delegate(
        &self,
        target: &crate::domain::AgentId,
    ) -> Result<(DelegateMessage, Acknowledgement), QueueError> {
        self.inner.receive_delegate(target)
    }

    fn receive_delegate_reply(
        &self,
        reply_key: &crate::domain::RoutingKey,
    ) -> Result<(CompleteMessage, Acknowledgement), QueueError> {
        self.inner.receive_delegate_reply(reply_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AgentId, DagNode, DelegateDiagnostics, HarnessType, InMemoryDagStore, InferenceBackendType,
        InvokeDiagnostics, MessageType, Nonce, Operation, RequestDiagnostics, RuntimeDiagnostics,
        RuntimeType, Sequence, ServiceBackend, ServiceDiagnostics, SessionId, SubmissionId,
        TimelineId,
    };
    use crate::queue::InMemoryQueue;

    fn test_store() -> Arc<dyn DagStore> {
        Arc::new(InMemoryDagStore::new())
    }

    fn test_queue(store: Arc<dyn DagStore>) -> RecordingQueue {
        let inner: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        RecordingQueue::new(inner, store)
    }

    fn test_session() -> SessionId {
        SessionId::from("ses-test-001".to_string())
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-001".to_string())
    }

    fn test_agent_id() -> AgentId {
        AgentId::new("echo")
    }

    fn test_invoke() -> InvokeMessage {
        InvokeMessage::new(
            TimelineId::main(),
            test_submission(),
            test_session(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"hello".to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
                history_turns: 0,
            },
            String::new(),
        )
    }

    fn test_request() -> RequestMessage {
        RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            test_session(),
            test_agent_id(),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
            Operation::Run,
            Sequence::first(),
            b"prompt".to_vec(),
            None,
            RequestDiagnostics {
                sequence: 1,
                endpoint: "/test".to_string(),
                request_bytes: 6,
                received_at_ms: 0,
            },
        )
    }

    fn test_response(request: &RequestMessage) -> ResponseMessage {
        ResponseMessage::from_request_with_diagnostics(
            request,
            b"answer".to_vec(),
            ServiceDiagnostics::placeholder(),
        )
    }

    fn test_complete() -> CompleteMessage {
        CompleteMessage::new(
            TimelineId::main(),
            test_submission(),
            test_session(),
            test_agent_id(),
            HarnessType::Cli,
            b"done".to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        )
    }

    fn test_delegate() -> DelegateMessage {
        DelegateMessage::new(
            TimelineId::main(),
            test_submission(),
            test_session(),
            AgentId::new("echo"),
            AgentId::new("summarizer"),
            b"delegate this".to_vec(),
            Nonce::new("test-nonce"),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        )
    }

    #[test]
    fn send_invoke_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let msg = test_invoke();
        let session_id = msg.session.as_str().to_string();

        queue.send_invoke(msg).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Invoke);
        assert_eq!(nodes[0].parent_hash, "");
    }

    #[test]
    fn send_request_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let msg = test_request();
        let session_id = msg.session.as_str().to_string();

        queue.send_request(msg).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Request);
    }

    #[test]
    fn send_response_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let request = test_request();
        let msg = test_response(&request);
        let session_id = msg.session.as_str().to_string();

        queue.send_response(msg).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Response);
    }

    #[test]
    fn send_complete_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let msg = test_complete();
        let session_id = msg.session.as_str().to_string();

        queue.send_complete(msg).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Complete);
    }

    #[test]
    fn send_delegate_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let msg = test_delegate();
        let session_id = msg.session.as_str().to_string();

        queue.send_delegate(msg).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Delegate);
    }

    #[test]
    fn merkle_chain_links_sequential_messages() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let invoke = test_invoke();
        let session_id = invoke.session.as_str().to_string();

        let mut request = test_request();
        // Use same session
        request.session = SessionId::from(session_id.clone());

        queue.send_invoke(invoke).unwrap();
        queue.send_request(request).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 2);
        // Second node's parent should be first node's hash
        assert_eq!(nodes[1].parent_hash, nodes[0].hash);
    }

    #[test]
    fn different_sessions_chain_independently() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        // Use different payloads — the content hash covers payload but not
        // session_id, so identical payloads produce the same hash and
        // INSERT OR IGNORE deduplicates.
        let mut invoke1 = test_invoke();
        invoke1.session = SessionId::from("ses-aaa".to_string());
        invoke1.payload = b"hello-aaa".to_vec();
        let session1 = invoke1.session.as_str().to_string();

        let mut invoke2 = test_invoke();
        invoke2.session = SessionId::from("ses-bbb".to_string());
        invoke2.payload = b"hello-bbb".to_vec();
        let session2 = invoke2.session.as_str().to_string();

        queue.send_invoke(invoke1).unwrap();
        queue.send_invoke(invoke2).unwrap();

        let nodes1 = store.get_session_nodes(&session1).unwrap();
        let nodes2 = store.get_session_nodes(&session2).unwrap();

        assert_eq!(nodes1.len(), 1);
        assert_eq!(nodes2.len(), 1);
        // Both are root nodes (no parent)
        assert_eq!(nodes1[0].parent_hash, "");
        assert_eq!(nodes2[0].parent_hash, "");
    }

    #[test]
    fn receive_methods_delegate_through() {
        let store = test_store();
        let inner = Arc::new(InMemoryQueue::new());
        let queue = RecordingQueue::new(
            Arc::clone(&inner) as Arc<dyn MessageQueue + Send + Sync>,
            store,
        );

        // Send a message through the inner queue's trait method
        let msg = test_invoke();
        inner.send_invoke(msg).unwrap();

        // Receive through the recording queue — should delegate to inner
        let result = queue.receive_invoke(&test_agent_id());
        assert!(result.is_ok());
    }

    #[test]
    fn dag_store_error_does_not_block_send() {
        // Use a store that always fails on insert
        struct FailStore;
        impl DagStore for FailStore {
            fn insert_node(&self, _: &DagNode) -> Result<(), String> {
                Err("simulated failure".to_string())
            }
            fn get_node(&self, _: &str) -> Result<Option<DagNode>, String> {
                Ok(None)
            }
            fn get_session_nodes(&self, _: &str) -> Result<Vec<DagNode>, String> {
                Ok(vec![])
            }
            fn get_children(&self, _: &str) -> Result<Vec<DagNode>, String> {
                Ok(vec![])
            }
            fn latest_state(&self, _: &str) -> Result<Option<String>, String> {
                Ok(None)
            }
            fn latest_node_hash(&self, _: &str) -> Result<Option<String>, String> {
                Ok(None)
            }
            fn set_checkout_state(&self, _: &str, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn create_timeline(
                &self,
                _: &str,
                _: &str,
                _: Option<i64>,
                _: Option<&str>,
            ) -> Result<i64, String> {
                Ok(0)
            }
            fn get_timeline_by_branch(
                &self,
                _: &str,
            ) -> Result<Option<crate::domain::Timeline>, String> {
                Ok(None)
            }
            fn get_timeline(&self, _: i64) -> Result<Option<crate::domain::Timeline>, String> {
                Ok(None)
            }
            fn seal_timeline(&self, _: i64) -> Result<(), String> {
                Ok(())
            }
            fn rename_timeline(&self, _: i64, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn is_timeline_sealed(&self, _: i64) -> Result<bool, String> {
                Ok(false)
            }
            fn list_sessions(&self) -> Result<Vec<crate::domain::SessionSummary>, String> {
                Ok(vec![])
            }
        }

        let store: Arc<dyn DagStore> = Arc::new(FailStore);
        let inner: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let queue = RecordingQueue::new(inner, store);

        // Send should still succeed despite store failure
        let result = queue.send_invoke(test_invoke());
        assert!(result.is_ok());
    }

    #[test]
    fn invoke_with_dag_parent_overrides_chain() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        // Send a normal invoke first to populate the chain
        let invoke1 = test_invoke();
        let session_id = invoke1.session.as_str().to_string();
        queue.send_invoke(invoke1).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 1);
        let first_hash = nodes[0].hash.clone();

        // Send a second invoke (same session) — normally chains off first
        let mut invoke2 = test_invoke();
        invoke2.payload = b"second".to_vec();
        queue.send_invoke(invoke2).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[1].parent_hash, first_hash, "normal chaining");

        // Send a third invoke with explicit dag_parent pointing to first node,
        // bypassing the chain cache (which would point to second node)
        let mut invoke3 = test_invoke();
        invoke3.payload = b"forked".to_vec();
        invoke3.dag_parent = first_hash.clone();
        queue.send_invoke(invoke3).unwrap();

        let nodes = store.get_session_nodes(&session_id).unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(
            nodes[2].parent_hash, first_hash,
            "dag_parent should override chain cache"
        );
    }
}
