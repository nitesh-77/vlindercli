//! DAG worker helpers — convert `ObservableMessage` to `DagNode` for storage
//! (ADR 065, 067, 078, 080).
//!
//! `SQLite` recording is handled synchronously by the transactional outbox
//! (`RecordingQueue`, ADR 080). The `dag-git` NATS consumer reconstructs
//! messages via `vlinder-nats` and passes them here for `DagNode` conversion.

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
        msg_type: msg.message_type(),
        session: msg.session().clone(),
        submission: msg.submission().clone(),
        branch: *msg.branch(),
        protocol_version: msg.protocol_version().to_string(),
        message: Some(msg.clone()),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AgentName, BranchId, DagStore, DelegateDiagnostics, DelegateMessage, DelegateReplyMessage,
        HarnessType, InMemoryDagStore, MessageType, Nonce, RuntimeDiagnostics, SessionId,
        SubmissionId,
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
        DelegateReplyMessage::new(
            BranchId::from(1),
            submission(),
            session(),
            AgentName::new("myagent"),
            HarnessType::Cli,
            payload.to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        )
        .into()
    }

    fn test_complete(payload: &[u8], state: Option<String>) -> ObservableMessage {
        DelegateReplyMessage::new(
            BranchId::from(1),
            submission(),
            session(),
            AgentName::new("myagent"),
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
            AgentName::new("coordinator"),
            AgentName::new("summarizer"),
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
        assert_eq!(node.message_type(), MessageType::Complete);
        let (from, to) = node.message.as_ref().unwrap().sender_receiver();
        assert_eq!(from, "myagent");
        assert_eq!(to, "cli");
        assert_eq!(*node.session_id(), session());
        assert_eq!(*node.submission_id(), submission());
        assert_eq!(node.payload(), b"invoke-payload");
        assert_eq!(node.parent_id, DagNodeId::root());
        assert!(!node.id.as_str().is_empty());
    }

    #[test]
    fn build_dag_node_from_delegate() {
        let msg = test_delegate(b"delegate-payload");
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message_type(), MessageType::Delegate);
        let (from, to) = node.message.as_ref().unwrap().sender_receiver();
        assert_eq!(from, "coordinator");
        assert_eq!(to, "summarizer");
    }

    #[test]
    fn build_dag_node_checkpoint_none_on_invoke() {
        let msg = test_invoke(b"payload");
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message.as_ref().unwrap().checkpoint(), None);
    }

    #[test]
    fn build_dag_node_state_preserved() {
        let msg = test_complete(b"done", Some("state-hash".to_string()));
        let root = DagNodeId::root();
        let node = build_dag_node(&msg, &root, &Snapshot::empty());
        assert_eq!(node.message.as_ref().unwrap().state(), Some("state-hash"));
    }

    // --- DagNode accessors on invoke ---

    #[test]
    fn dag_node_accessors_on_invoke() {
        let session = session();
        let sub = submission();

        let node = DagNode {
            id: DagNodeId::from("test-hash".to_string()),
            parent_id: DagNodeId::root(),
            created_at: chrono::Utc::now(),
            state: Snapshot::empty(),
            msg_type: MessageType::Invoke,
            session: session.clone(),
            submission: sub.clone(),
            branch: BranchId::from(1),
            protocol_version: "v1".to_string(),
            message: None,
        };

        assert_eq!(node.message_type(), MessageType::Invoke);
        assert_eq!(*node.session_id(), session);
        assert_eq!(*node.submission_id(), sub);
        assert_eq!(*node.branch_id(), BranchId::from(1));
        // message: None for typed-table invoke rows — payload returns empty
        assert_eq!(node.payload(), b"");
        assert_eq!(node.protocol_version(), "v1");
    }

    #[test]
    fn dag_node_message_none_returns_empty_payload() {
        // A DagNode can have `message: None` for typed-table rows (invoke).
        // This is not an error — payload() simply returns empty.
        let node = DagNode {
            id: DagNodeId::from("empty".to_string()),
            parent_id: DagNodeId::root(),
            created_at: chrono::Utc::now(),
            state: Snapshot::empty(),
            msg_type: MessageType::Invoke,
            session: session(),
            submission: submission(),
            branch: BranchId::from(1),
            protocol_version: "v1".to_string(),
            message: None,
        };
        assert_eq!(node.payload(), b"");
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
        assert_eq!(nodes[0].message_type(), MessageType::Complete);
        let (from, to) = nodes[0].message.as_ref().unwrap().sender_receiver();
        assert_eq!(from, "myagent");
        assert_eq!(to, "cli");
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
        let msg2: ObservableMessage = DelegateReplyMessage::new(
            BranchId::from(1),
            submission_alt(),
            sess2.clone(),
            AgentName::new("agent-b"),
            HarnessType::Cli,
            b"sess2-first".to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
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
