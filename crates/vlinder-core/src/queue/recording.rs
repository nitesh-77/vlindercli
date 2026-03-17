//! RecordingQueue — transactional outbox for synchronous DAG recording.
//!
//! Wraps any `MessageQueue` and records a `DagNode` into a `DagStore`
//! before forwarding each send. This eliminates the race condition where
//! a query sees stale state because the async NATS consumer hasn't
//! processed the latest messages yet.
//!
//! Receive and routing methods delegate straight through.

use std::sync::Arc;

use crate::domain::workers::dag::build_dag_node;
use crate::domain::{
    Acknowledgement, CompleteMessage, DagNodeId, DagStore, DelegateMessage, ForkMessage,
    InvokeMessage, MessageQueue, ObservableMessage, PromoteMessage, QueueError, RepairMessage,
    RequestMessage, ResponseMessage, Snapshot, SubmissionId,
};

/// A `MessageQueue` decorator that synchronously records DAG nodes on send.
///
/// Every `send_*` call: clone → convert to `ObservableMessage` → build
/// `DagNode` with Merkle chaining → insert into `DagStore` → forward
/// original message to inner queue.
///
/// DagStore write failures are logged but don't block message sending.
///
/// Merkle chain parent resolution uses the timeline `head` pointer stored
/// in the database, ensuring that multiple RecordingQueue instances (sidecar
/// + daemon) share a single source of truth for chaining (issue #37).
pub struct RecordingQueue {
    inner: Arc<dyn MessageQueue + Send + Sync>,
    store: Arc<dyn DagStore>,
}

impl RecordingQueue {
    pub fn new(inner: Arc<dyn MessageQueue + Send + Sync>, store: Arc<dyn DagStore>) -> Self {
        Self { inner, store }
    }

    /// Record a DAG node for the given observable message.
    fn record(&self, observable: &ObservableMessage) {
        let branch_id = *observable.branch();

        // Explicit dag_parent on Invoke/Fork overrides the latest node on the timeline.
        let dag_parent_override: Option<DagNodeId> = match observable {
            ObservableMessage::Invoke(m) if !m.dag_parent.is_empty() => Some(m.dag_parent.clone()),
            ObservableMessage::Fork(m) => Some(m.fork_point.clone()),
            _ => None,
        };

        // Look up parent node: dag_parent override → latest node on branch
        let parent_node = dag_parent_override
            .and_then(|id| {
                self.store.get_node(&id).unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "Failed to look up dag_parent node");
                    None
                })
            })
            .or_else(|| {
                self.store
                    .latest_node_on_branch(branch_id, None)
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, branch = branch_id.as_i64(), "Failed to query latest node on branch");
                        None
                    })
            });

        let parent_id = parent_node
            .as_ref()
            .map(|n| n.id.clone())
            .unwrap_or_else(DagNodeId::root);
        let parent_state = parent_node
            .as_ref()
            .map(|n| &n.state)
            .cloned()
            .unwrap_or_else(Snapshot::empty);

        let node = build_dag_node(observable, &parent_id, &parent_state);
        let node_id = node.id.clone();

        if let Err(e) = self.store.insert_node(&node) {
            tracing::warn!(error = %node_id, "Failed to record DAG node (outbox): {}", e);
        }
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

    // -------------------------------------------------------------------------
    // Repair methods — record + forward on send, delegate on receive
    // -------------------------------------------------------------------------

    fn send_repair(&self, msg: RepairMessage) -> Result<(), QueueError> {
        self.record(&msg.clone().into());
        self.inner.send_repair(msg)
    }

    fn receive_repair(
        &self,
        agent: &crate::domain::AgentId,
    ) -> Result<(RepairMessage, Acknowledgement), QueueError> {
        self.inner.receive_repair(agent)
    }

    // -------------------------------------------------------------------------
    // Fork methods — record + forward on send
    // -------------------------------------------------------------------------

    fn send_fork(&self, msg: ForkMessage) -> Result<(), QueueError> {
        self.record(&msg.clone().into());

        // Create the branch row so `--branch` and `session fork` can find it.
        match self
            .store
            .create_branch(&msg.branch_name, &msg.session, Some(&msg.fork_point))
        {
            Ok(id) => {
                tracing::info!(
                    branch_id = id.as_i64(),
                    branch = %msg.branch_name,
                    "Created branch on fork"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    branch = %msg.branch_name,
                    "Failed to create branch on fork"
                );
            }
        }

        self.inner.send_fork(msg)
    }

    fn send_promote(&self, msg: PromoteMessage) -> Result<(), QueueError> {
        self.record(&msg.clone().into());

        // Promote the branch: seal old main, rename promoted branch to "main".
        let branch_to_promote = self.store.get_branch(msg.branch).ok().flatten();

        let old_main = self
            .store
            .get_branch_by_name("main")
            .ok()
            .flatten()
            .filter(|b| b.session_id == msg.session);

        if let Some(old) = old_main {
            let sealed_name = format!("broken-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
            if let Err(e) = self.store.seal_branch(old.id, chrono::Utc::now()) {
                tracing::warn!(error = %e, branch = old.id.as_i64(), "Failed to seal old main");
            }
            if let Err(e) = self.store.rename_branch(old.id, &sealed_name) {
                tracing::warn!(error = %e, branch = old.id.as_i64(), "Failed to rename old main");
            }
            tracing::info!(
                old_main_id = old.id.as_i64(),
                sealed_name = %sealed_name,
                "Sealed old main branch"
            );
        }

        if let Some(promoted) = branch_to_promote {
            if let Err(e) = self.store.rename_branch(promoted.id, "main") {
                tracing::warn!(error = %e, branch = promoted.id.as_i64(), "Failed to rename promoted branch to main");
            }
            if let Err(e) = self
                .store
                .update_session_default_branch(&msg.session, promoted.id)
            {
                tracing::warn!(error = %e, "Failed to update session default branch");
            }
            tracing::info!(
                branch_id = promoted.id.as_i64(),
                old_name = %promoted.name,
                "Promoted branch to main"
            );
        }

        self.inner.send_promote(msg)
    }

    fn send_session_start(
        &self,
        msg: crate::domain::SessionStartMessage,
    ) -> Result<crate::domain::BranchId, QueueError> {
        // Create the default "main" branch, then the session pointing to it.
        let default_branch = self
            .store
            .create_branch("main", &msg.session, None)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Failed to create default branch");
                crate::domain::BranchId::from(1) // fallback
            });
        let session =
            crate::domain::Session::new(msg.session.clone(), &msg.agent_name, default_branch);
        if let Err(e) = self.store.create_session(&session) {
            tracing::warn!(
                error = %e,
                session = %msg.session.as_str(),
                "Failed to persist session"
            );
        }
        let _ = self.inner.send_session_start(msg);
        Ok(default_branch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AgentId, BranchId, DagNode, DelegateDiagnostics, HarnessType, InMemoryDagStore,
        InferenceBackendType, InvokeDiagnostics, MessageType, Nonce, Operation, RequestDiagnostics,
        RuntimeDiagnostics, RuntimeType, Sequence, ServiceBackend, ServiceDiagnostics, SessionId,
        SubmissionId,
    };
    use crate::queue::InMemoryQueue;

    fn test_store() -> Arc<dyn DagStore> {
        let store = Arc::new(InMemoryDagStore::new());
        // Seed "main" branch (id=1) — mirrors production setup.
        store.create_branch("main", &test_session(), None).unwrap();
        store
    }

    fn test_queue(store: Arc<dyn DagStore>) -> RecordingQueue {
        let inner: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        RecordingQueue::new(inner, store)
    }

    fn test_session() -> SessionId {
        SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap()
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-001".to_string())
    }

    fn test_agent_id() -> AgentId {
        AgentId::new("echo")
    }

    fn test_invoke() -> InvokeMessage {
        InvokeMessage::new(
            BranchId::from(1),
            test_submission(),
            test_session(),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"hello".to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        )
    }

    fn test_request() -> RequestMessage {
        RequestMessage::new(
            BranchId::from(1),
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
            BranchId::from(1),
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
            BranchId::from(1),
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
        let sid = msg.session.clone();

        queue.send_invoke(msg).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type(), MessageType::Invoke);
        assert_eq!(nodes[0].parent_id, DagNodeId::root());
    }

    #[test]
    fn send_request_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let msg = test_request();
        let sid = msg.session.clone();

        queue.send_request(msg).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type(), MessageType::Request);
    }

    #[test]
    fn send_response_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let request = test_request();
        let msg = test_response(&request);
        let sid = msg.session.clone();

        queue.send_response(msg).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type(), MessageType::Response);
    }

    #[test]
    fn send_complete_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let msg = test_complete();
        let sid = msg.session.clone();

        queue.send_complete(msg).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type(), MessageType::Complete);
    }

    #[test]
    fn send_delegate_records_dag_node() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let msg = test_delegate();
        let sid = msg.session.clone();

        queue.send_delegate(msg).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type(), MessageType::Delegate);
    }

    #[test]
    fn merkle_chain_links_sequential_messages() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        let invoke = test_invoke();
        let sid = invoke.session.clone();

        let mut request = test_request();
        // Use same session
        request.session = sid.clone();

        queue.send_invoke(invoke).unwrap();
        queue.send_request(request).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 2);
        // Second node's parent should be first node's id
        assert_eq!(nodes[1].parent_id, nodes[0].id);
    }

    #[test]
    fn same_timeline_chains_across_sessions() {
        let store = test_store();
        let queue = test_queue(Arc::clone(&store));

        // Two sessions on the same timeline chain sequentially via the
        // shared timeline head pointer.
        let ses_aaa =
            SessionId::try_from("e2660cff-33d6-4428-acca-2d297dcc1cad".to_string()).unwrap();
        let ses_bbb =
            SessionId::try_from("7897b0a7-937b-4457-87c3-07c4cab30c55".to_string()).unwrap();

        let mut invoke1 = test_invoke();
        invoke1.session = ses_aaa.clone();
        invoke1.payload = b"hello-aaa".to_vec();

        let mut invoke2 = test_invoke();
        invoke2.session = ses_bbb.clone();
        invoke2.payload = b"hello-bbb".to_vec();

        queue.send_invoke(invoke1).unwrap();
        queue.send_invoke(invoke2).unwrap();

        let nodes1 = store.get_session_nodes(&ses_aaa).unwrap();
        let nodes2 = store.get_session_nodes(&ses_bbb).unwrap();

        assert_eq!(nodes1.len(), 1);
        assert_eq!(nodes2.len(), 1);
        // First invoke is root, second chains off first via timeline head
        assert_eq!(nodes1[0].parent_id, DagNodeId::root());
        assert_eq!(nodes2[0].parent_id, nodes1[0].id);
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
            fn get_node(&self, _: &crate::domain::DagNodeId) -> Result<Option<DagNode>, String> {
                Ok(None)
            }
            fn get_session_nodes(
                &self,
                _: &crate::domain::SessionId,
            ) -> Result<Vec<DagNode>, String> {
                Ok(vec![])
            }
            fn get_children(&self, _: &crate::domain::DagNodeId) -> Result<Vec<DagNode>, String> {
                Ok(vec![])
            }
            fn create_branch(
                &self,
                _: &str,
                _: &crate::domain::SessionId,
                _: Option<&crate::domain::DagNodeId>,
            ) -> Result<crate::domain::BranchId, String> {
                Ok(crate::domain::BranchId::from(0))
            }
            fn get_branch_by_name(&self, _: &str) -> Result<Option<crate::domain::Branch>, String> {
                Ok(None)
            }
            fn get_branch(
                &self,
                _: crate::domain::BranchId,
            ) -> Result<Option<crate::domain::Branch>, String> {
                Ok(None)
            }
            fn list_sessions(&self) -> Result<Vec<crate::domain::SessionSummary>, String> {
                Ok(vec![])
            }
            fn get_nodes_by_submission(&self, _: &str) -> Result<Vec<DagNode>, String> {
                Ok(vec![])
            }
            fn get_branches_for_session(
                &self,
                _: &crate::domain::SessionId,
            ) -> Result<Vec<crate::domain::Branch>, String> {
                Ok(vec![])
            }
            fn latest_node_on_branch(
                &self,
                _: crate::domain::BranchId,
                _: Option<crate::domain::MessageType>,
            ) -> Result<Option<crate::domain::DagNode>, String> {
                Ok(None)
            }
            fn create_session(&self, _: &crate::domain::Session) -> Result<(), String> {
                Ok(())
            }
            fn get_session(
                &self,
                _: &crate::domain::SessionId,
            ) -> Result<Option<crate::domain::Session>, String> {
                Ok(None)
            }
            fn get_session_by_name(
                &self,
                _: &str,
            ) -> Result<Option<crate::domain::Session>, String> {
                Ok(None)
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
        let sid = invoke1.session.clone();
        queue.send_invoke(invoke1).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 1);
        let first_id = nodes[0].id.clone();

        // Send a second invoke (same session) — normally chains off first
        let mut invoke2 = test_invoke();
        invoke2.payload = b"second".to_vec();
        queue.send_invoke(invoke2).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[1].parent_id, first_id, "normal chaining");

        // Send a third invoke with explicit dag_parent pointing to first node,
        // bypassing the chain cache (which would point to second node)
        let mut invoke3 = test_invoke();
        invoke3.payload = b"forked".to_vec();
        invoke3.dag_parent = first_id.clone();
        queue.send_invoke(invoke3).unwrap();

        let nodes = store.get_session_nodes(&sid).unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(
            nodes[2].parent_id, first_id,
            "dag_parent should override chain cache"
        );
    }
}
