//! DAG worker helpers — convert ObservableMessage to DagNode for storage
//! (ADR 065, 067, 078, 080).
//!
//! SQLite recording is handled synchronously by the transactional outbox
//! (RecordingQueue, ADR 080). The `dag-git` NATS consumer reconstructs
//! messages via `vlinder-nats` and passes them here for DagNode conversion.

use chrono::Utc;

use crate::domain::message::ObservableMessage;
use crate::domain::{hash_dag_node, DagNode, DagNodeId, Instance, Snapshot, StateHash};

/// Build a `DagNode` from an `ObservableMessage` and Merkle chain state.
///
/// Computes the content-addressed hash and wraps the message with DAG metadata.
/// The snapshot is initialized from the message's state if present, otherwise
/// empty. Callers that have the parent node should call `with_parent_snapshot`
/// to inherit unchanged store states.
pub fn build_dag_node(
    msg: &ObservableMessage,
    parent_id: &DagNodeId,
    parent_state: &Snapshot,
) -> DagNode {
    let id = hash_dag_node(
        msg.payload(),
        parent_id,
        &msg.message_type(),
        &msg.diagnostics_json(),
        msg.session(),
    );

    // Inherit parent's snapshot, updating the KV store entry if this message
    // carries a state hash.
    let state = match msg.state() {
        Some(s) if !s.is_empty() => {
            parent_state.with_state(Instance::from("kv"), StateHash::from(s.to_string()))
        }
        _ => parent_state.clone(),
    };

    DagNode {
        id,
        parent_id: parent_id.clone(),
        created_at: Utc::now(),
        state,
        message: msg.clone(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AgentId, BranchId, CompleteMessage, DagStore, DelegateDiagnostics, DelegateMessage,
        HarnessType, InMemoryDagStore, InferenceBackendType, InvokeDiagnostics, InvokeMessage,
        MessageType, Nonce, Operation, RequestDiagnostics, RequestMessage, ResponseMessage,
        RuntimeDiagnostics, RuntimeType, Sequence, ServiceBackend, SessionId, SubmissionId,
    };

    fn session() -> SessionId {
        SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap()
    }
    fn submission() -> SubmissionId {
        SubmissionId::from("sub-1".to_string())
    }
    fn submission_alt() -> SubmissionId {
        SubmissionId::from("sub-2".to_string())
    }

    fn test_invoke(payload: &[u8]) -> ObservableMessage {
        InvokeMessage::new(
            BranchId::from(1),
            submission(),
            session(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("myagent"),
            payload.to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        )
        .into()
    }

    fn test_request(payload: &[u8]) -> ObservableMessage {
        RequestMessage::new(
            BranchId::from(1),
            submission(),
            session(),
            AgentId::new("myagent"),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
            Operation::Run,
            Sequence::first(),
            payload.to_vec(),
            None,
            RequestDiagnostics {
                sequence: 1,
                endpoint: "/infer".to_string(),
                request_bytes: 0,
                received_at_ms: 0,
            },
        )
        .into()
    }

    fn test_response(payload: &[u8]) -> ObservableMessage {
        let request = RequestMessage::new(
            BranchId::from(1),
            submission(),
            session(),
            AgentId::new("myagent"),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
            Operation::Run,
            Sequence::first(),
            b"prompt".to_vec(),
            None,
            RequestDiagnostics {
                sequence: 1,
                endpoint: "/infer".to_string(),
                request_bytes: 0,
                received_at_ms: 0,
            },
        );
        ResponseMessage::from_request(&request, payload.to_vec()).into()
    }

    fn test_complete(payload: &[u8], state: Option<String>) -> ObservableMessage {
        CompleteMessage::new(
            BranchId::from(1),
            submission(),
            session(),
            AgentId::new("myagent"),
            HarnessType::Cli,
            payload.to_vec(),
            state,
            RuntimeDiagnostics::placeholder(0),
        )
        .into()
    }

    fn test_delegate(payload: &[u8]) -> ObservableMessage {
        DelegateMessage::new(
            BranchId::from(1),
            submission(),
            session(),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            payload.to_vec(),
            Nonce::new("nonce-1"),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        )
        .into()
    }

    // --- ObservableMessage → DagNode conversion tests ---

    #[test]
    fn build_dag_node_from_invoke() {
        let msg = test_invoke(b"invoke-payload");
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message_type(), MessageType::Invoke);
        let (from, to) = node.message.from_to();
        assert_eq!(from, "cli");
        assert_eq!(to, "myagent");
        assert_eq!(*node.session_id(), session());
        assert_eq!(*node.submission_id(), submission());
        assert_eq!(node.payload(), b"invoke-payload");
        assert_eq!(node.parent_id, DagNodeId::root());
        assert!(!node.id.as_str().is_empty());
    }

    #[test]
    fn build_dag_node_from_request() {
        let msg = test_request(b"request-payload");
        let parent = DagNodeId::from("parent-abc".to_string());
        let node = build_dag_node(&msg, &parent, &Snapshot::empty());
        assert_eq!(node.message_type(), MessageType::Request);
        let (from, to) = node.message.from_to();
        assert_eq!(from, "myagent");
        assert_eq!(to, "infer.ollama");
        assert_eq!(node.parent_id, parent);
    }

    #[test]
    fn build_dag_node_from_response() {
        let msg = test_response(b"response-payload");
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message_type(), MessageType::Response);
        let (from, to) = node.message.from_to();
        assert_eq!(from, "infer.ollama");
        assert_eq!(to, "myagent");
    }

    #[test]
    fn build_dag_node_from_delegate() {
        let msg = test_delegate(b"delegate-payload");
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message_type(), MessageType::Delegate);
        let (from, to) = node.message.from_to();
        assert_eq!(from, "coordinator");
        assert_eq!(to, "summarizer");
    }

    #[test]
    fn build_dag_node_checkpoint_preserved_on_request() {
        let mut msg = test_request(b"prompt");
        if let ObservableMessage::Request(ref mut m) = msg {
            m.checkpoint = Some("summarize".to_string());
        }
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message.checkpoint(), Some("summarize"));
    }

    #[test]
    fn build_dag_node_checkpoint_none_on_invoke() {
        let msg = test_invoke(b"payload");
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message.checkpoint(), None);
    }

    #[test]
    fn build_dag_node_state_preserved() {
        let msg = test_complete(b"done", Some("state-hash".to_string()));
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message.state(), Some("state-hash"));
    }

    // --- Integration: ObservableMessage → DagNode → store round-trip ---

    fn test_store() -> InMemoryDagStore {
        InMemoryDagStore::new()
    }

    #[test]
    fn store_invoke_dag_node() {
        let store = test_store();
        let msg = test_invoke(b"invoke-payload");
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        store.insert_node(&node).unwrap();

        let nodes = store.get_session_nodes(&session()).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type(), MessageType::Invoke);
        let (from, to) = nodes[0].message.from_to();
        assert_eq!(from, "cli");
        assert_eq!(to, "myagent");
    }

    #[test]
    fn messages_chain_in_session() {
        let store = test_store();
        let mut last_id = DagNodeId::root();

        let msg1 = test_invoke(b"first");
        let node1 = build_dag_node(&msg1, &last_id, &Snapshot::empty());
        last_id = node1.id.clone();
        store.insert_node(&node1).unwrap();

        let msg2 = test_request(b"second");
        let node2 = build_dag_node(&msg2, &last_id, &node1.state);
        last_id = node2.id.clone();
        store.insert_node(&node2).unwrap();

        let msg3 = test_response(b"third");
        let node3 = build_dag_node(&msg3, &last_id, &node2.state);
        store.insert_node(&node3).unwrap();

        let nodes = store.get_session_nodes(&session()).unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].parent_id, DagNodeId::root());
        assert_eq!(nodes[1].parent_id, nodes[0].id);
        assert_eq!(nodes[2].parent_id, nodes[1].id);
    }

    #[test]
    fn different_sessions_chain_independently() {
        let store = test_store();

        let msg1 = test_invoke(b"sess1-first");
        let root = DagNodeId::root();
        let node1 = build_dag_node(&msg1, &root, &Snapshot::empty());
        store.insert_node(&node1).unwrap();

        // Different session
        let sess2 =
            SessionId::try_from("e2660cff-33d6-4428-acca-2d297dcc1cad".to_string()).unwrap();
        let msg2: ObservableMessage = InvokeMessage::new(
            BranchId::from(1),
            submission_alt(),
            sess2.clone(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("agent-b"),
            b"sess2-first".to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        )
        .into();
        let node2 = build_dag_node(&msg2, &root, &Snapshot::empty());
        store.insert_node(&node2).unwrap();

        let msg3 = test_complete(b"sess1-second", None);
        let node3 = build_dag_node(&msg3, &node1.id, &node1.state);
        store.insert_node(&node3).unwrap();

        let sess1_nodes = store.get_session_nodes(&session()).unwrap();
        let sess2_nodes = store.get_session_nodes(&sess2).unwrap();

        assert_eq!(sess1_nodes.len(), 2);
        assert_eq!(sess2_nodes.len(), 1);
        assert_eq!(sess1_nodes[1].parent_id, sess1_nodes[0].id);
        assert_eq!(sess2_nodes[0].parent_id, DagNodeId::root());
    }
}
