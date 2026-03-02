//! DAG worker helpers — convert ObservableMessage to DagNode for storage
//! (ADR 065, 067, 078, 080).
//!
//! SQLite recording is handled synchronously by the transactional outbox
//! (RecordingQueue, ADR 080). The `dag-git` NATS consumer reconstructs
//! messages via `vlinder-nats` and passes them here for DagNode conversion.

use chrono::Utc;

use crate::domain::message::ObservableMessage;
use crate::domain::{hash_dag_node, DagNode};

// ============================================================================
// ObservableMessage → DagNode conversion helpers
// ============================================================================

/// Extract (from, to) routing pair from a reconstructed message.
pub fn observable_from_to(msg: &ObservableMessage) -> (String, String) {
    match msg {
        ObservableMessage::Invoke(m) => (m.harness.as_str().to_string(), m.agent_id.to_string()),
        ObservableMessage::Request(m) => (
            m.agent_id.to_string(),
            format!("{}.{}", m.service.service_type(), m.service.backend_str()),
        ),
        ObservableMessage::Response(m) => (
            format!("{}.{}", m.service.service_type(), m.service.backend_str()),
            m.agent_id.to_string(),
        ),
        ObservableMessage::Complete(m) => (m.agent_id.to_string(), m.harness.as_str().to_string()),
        ObservableMessage::Delegate(m) => (m.caller.to_string(), m.target.to_string()),
    }
}

/// Serialize diagnostics from a reconstructed message to JSON bytes.
pub fn serialize_diagnostics(msg: &ObservableMessage) -> Vec<u8> {
    let json = match msg {
        ObservableMessage::Invoke(m) => serde_json::to_vec(&m.diagnostics),
        ObservableMessage::Request(m) => serde_json::to_vec(&m.diagnostics),
        ObservableMessage::Response(m) => serde_json::to_vec(&m.diagnostics),
        ObservableMessage::Complete(m) => serde_json::to_vec(&m.diagnostics),
        ObservableMessage::Delegate(m) => serde_json::to_vec(&m.diagnostics),
    };
    json.unwrap_or_default()
}

/// Extract stderr bytes from a Complete or Delegate message.
///
/// Stderr lives on `RuntimeDiagnostics` — only Complete and Delegate carry it.
/// Returns empty for other message types.
pub fn extract_typed_stderr(msg: &ObservableMessage) -> Vec<u8> {
    match msg {
        ObservableMessage::Complete(m) => m.diagnostics.stderr.clone(),
        ObservableMessage::Delegate(m) => m.diagnostics.runtime.stderr.clone(),
        _ => Vec::new(),
    }
}

/// Extract state from a reconstructed message.
pub fn observable_state(msg: &ObservableMessage) -> Option<String> {
    match msg {
        ObservableMessage::Invoke(m) => m.state.clone(),
        ObservableMessage::Request(m) => m.state.clone(),
        ObservableMessage::Response(m) => m.state.clone(),
        ObservableMessage::Complete(m) => m.state.clone(),
        ObservableMessage::Delegate(m) => m.state.clone(),
    }
}

/// Build a `DagNode` from an `ObservableMessage` and Merkle chain state.
///
/// `parent_hash` is the hash of the previous node in the same session
/// (empty string for the first message).
pub fn build_dag_node(msg: &ObservableMessage, parent_hash: &str) -> DagNode {
    let message_type = msg.message_type();
    let (from, to) = observable_from_to(msg);
    let diagnostics = serialize_diagnostics(msg);
    let stderr = extract_typed_stderr(msg);
    let state = observable_state(msg);
    let payload = msg.payload();
    let session_id = msg.session().as_str().to_string();
    let hash = hash_dag_node(
        payload,
        parent_hash,
        &message_type,
        &diagnostics,
        &session_id,
    );

    DagNode {
        hash,
        parent_hash: parent_hash.to_string(),
        message_type,
        from,
        to,
        session_id,
        submission_id: msg.submission().as_str().to_string(),
        payload: payload.to_vec(),
        diagnostics,
        stderr,
        created_at: Utc::now(),
        state,
        protocol_version: msg.protocol_version().to_string(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AgentId, CompleteMessage, DagStore, DelegateDiagnostics, DelegateMessage, HarnessType,
        InMemoryDagStore, InferenceBackendType, InvokeDiagnostics, InvokeMessage, MessageType,
        Nonce, ObjectStorageType, Operation, RequestDiagnostics, RequestMessage, ResponseMessage,
        RuntimeDiagnostics, RuntimeType, Sequence, ServiceBackend, SessionId, SubmissionId,
        TimelineId,
    };

    fn session() -> SessionId {
        SessionId::from("sess-1".to_string())
    }
    fn submission() -> SubmissionId {
        SubmissionId::from("sub-1".to_string())
    }
    fn submission_alt() -> SubmissionId {
        SubmissionId::from("sub-2".to_string())
    }

    fn test_invoke(payload: &[u8]) -> ObservableMessage {
        InvokeMessage::new(
            TimelineId::main(),
            submission(),
            session(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("myagent"),
            payload.to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
                history_turns: 0,
            },
        )
        .into()
    }

    fn test_request(payload: &[u8]) -> ObservableMessage {
        RequestMessage::new(
            TimelineId::main(),
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
            TimelineId::main(),
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
            TimelineId::main(),
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
            TimelineId::main(),
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
        let node = build_dag_node(&msg, "");
        assert_eq!(node.message_type, MessageType::Invoke);
        assert_eq!(node.from, "cli");
        assert_eq!(node.to, "myagent");
        assert_eq!(node.session_id, "sess-1");
        assert_eq!(node.submission_id, "sub-1");
        assert_eq!(node.payload, b"invoke-payload");
        assert_eq!(node.parent_hash, "");
        assert!(!node.hash.is_empty());
    }

    #[test]
    fn build_dag_node_from_request() {
        let msg = test_request(b"request-payload");
        let node = build_dag_node(&msg, "parent-abc");
        assert_eq!(node.message_type, MessageType::Request);
        assert_eq!(node.from, "myagent");
        assert_eq!(node.to, "infer.ollama");
        assert_eq!(node.parent_hash, "parent-abc");
    }

    #[test]
    fn build_dag_node_from_response() {
        let msg = test_response(b"response-payload");
        let node = build_dag_node(&msg, "");
        assert_eq!(node.message_type, MessageType::Response);
        assert_eq!(node.from, "infer.ollama");
        assert_eq!(node.to, "myagent");
    }

    #[test]
    fn build_dag_node_from_delegate() {
        let msg = test_delegate(b"delegate-payload");
        let node = build_dag_node(&msg, "");
        assert_eq!(node.message_type, MessageType::Delegate);
        assert_eq!(node.from, "coordinator");
        assert_eq!(node.to, "summarizer");
    }

    #[test]
    fn build_dag_node_state_preserved() {
        let msg = test_complete(b"done", Some("state-hash".to_string()));
        let node = build_dag_node(&msg, "");
        assert_eq!(node.state, Some("state-hash".to_string()));
    }

    // --- Integration: ObservableMessage → DagNode → store round-trip ---

    fn test_store() -> InMemoryDagStore {
        InMemoryDagStore::new()
    }

    #[test]
    fn store_invoke_dag_node() {
        let store = test_store();
        let msg = test_invoke(b"invoke-payload");
        let node = build_dag_node(&msg, "");
        store.insert_node(&node).unwrap();

        let nodes = store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Invoke);
        assert_eq!(nodes[0].from, "cli");
        assert_eq!(nodes[0].to, "myagent");
    }

    #[test]
    fn messages_chain_in_session() {
        let store = test_store();
        let mut last_hash = String::new();

        let msg1 = test_invoke(b"first");
        let node1 = build_dag_node(&msg1, &last_hash);
        last_hash = node1.hash.clone();
        store.insert_node(&node1).unwrap();

        let msg2 = test_request(b"second");
        let node2 = build_dag_node(&msg2, &last_hash);
        last_hash = node2.hash.clone();
        store.insert_node(&node2).unwrap();

        let msg3 = test_response(b"third");
        let node3 = build_dag_node(&msg3, &last_hash);
        store.insert_node(&node3).unwrap();

        let nodes = store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].parent_hash, "");
        assert_eq!(nodes[1].parent_hash, nodes[0].hash);
        assert_eq!(nodes[2].parent_hash, nodes[1].hash);
    }

    #[test]
    fn different_sessions_chain_independently() {
        let store = test_store();

        let msg1 = test_invoke(b"sess1-first");
        let node1 = build_dag_node(&msg1, "");
        store.insert_node(&node1).unwrap();

        // Different session
        let msg2: ObservableMessage = InvokeMessage::new(
            TimelineId::main(),
            submission_alt(),
            SessionId::from("sess-2".to_string()),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("agent-b"),
            b"sess2-first".to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
                history_turns: 0,
            },
        )
        .into();
        let node2 = build_dag_node(&msg2, "");
        store.insert_node(&node2).unwrap();

        let msg3 = test_complete(b"sess1-second", None);
        let node3 = build_dag_node(&msg3, &node1.hash);
        store.insert_node(&node3).unwrap();

        let sess1 = store.get_session_nodes("sess-1").unwrap();
        let sess2 = store.get_session_nodes("sess-2").unwrap();

        assert_eq!(sess1.len(), 2);
        assert_eq!(sess2.len(), 1);
        assert_eq!(sess1[1].parent_hash, sess1[0].hash);
        assert_eq!(sess2[0].parent_hash, "");
    }
}
