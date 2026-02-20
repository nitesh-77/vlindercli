//! DAG worker helpers — reconstruct typed messages from NATS wire format
//! and convert to DagNode for storage (ADR 065, 067, 078, 080).
//!
//! SQLite recording is handled synchronously by the transactional outbox
//! (RecordingQueue, ADR 080). The `dag-git` NATS consumer reconstructs
//! messages for git commit storage.
//!
//! This module provides the shared reconstruction and conversion functions
//! used by the git worker (wired in `worker.rs`) and RecordingQueue.

use std::collections::HashMap;

use chrono::Utc;

use crate::domain::{
    AgentId, CompleteMessage, ContainerDiagnostics, DagNode, DelegateDiagnostics,
    DelegateMessage, HarnessType, InvokeDiagnostics, InvokeMessage, MessageId, MessageType,
    Operation, RequestDiagnostics, RequestMessage, RequestPayload, ResponseMessage,
    ResponsePayload, RuntimeType, Sequence, ServiceBackend, ServiceDiagnostics, ServiceType,
    SessionId, SubmissionId, TimelineId, hash_dag_node,
};
use crate::domain::message::ObservableMessage;

// ============================================================================
// NATS → ObservableMessage reconstruction
// ============================================================================

/// Reconstruct a typed `ObservableMessage` from NATS wire format.
///
/// Reads the subject to determine message type, then reads headers
/// to reconstruct the full typed message. Returns `None` for
/// unrecognized subjects or missing required headers.
///
/// This is the inverse of the `send_*` methods in `NatsQueue` — it reads
/// the same headers that were written during publish.
pub fn reconstruct_observable_message(
    subject: &str,
    headers: &HashMap<String, String>,
    payload: &[u8],
) -> Option<ObservableMessage> {
    let segments: Vec<&str> = subject.split('.').collect();

    // Minimum: vlinder.<timeline>.<submission>.<type>...
    if segments.len() < 4 || segments[0] != "vlinder" {
        return None;
    }

    let msg_type_str = segments[3];

    match msg_type_str {
        "invoke" => reconstruct_invoke(headers, payload),
        "req" => reconstruct_request(headers, payload),
        "res" => reconstruct_response(headers, payload),
        "complete" => reconstruct_complete(headers, payload),
        "delegate" => reconstruct_delegate(headers, payload),
        _ => None,
    }
}

fn reconstruct_invoke(
    headers: &HashMap<String, String>,
    payload: &[u8],
) -> Option<ObservableMessage> {
    let diagnostics = headers.get("diagnostics")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| InvokeDiagnostics {
            harness_version: String::new(),
            history_turns: 0,
        });

    Some(ObservableMessage::Invoke(InvokeMessage {
        id: MessageId::from(headers.get("msg-id")?.clone()),
        protocol_version: headers.get("protocol-version").cloned().unwrap_or_default(),
        timeline: headers.get("timeline-id").map(|s| TimelineId::from(s.clone())).unwrap_or_else(TimelineId::main),
        submission: SubmissionId::from(headers.get("submission-id")?.clone()),
        session: SessionId::from(headers.get("session-id")?.clone()),
        harness: parse_harness(headers.get("harness")?)?,
        runtime: parse_runtime(headers.get("runtime")?)?,
        agent_id: AgentId::new(headers.get("agent-id")?),
        payload: payload.to_vec(),
        state: headers.get("state").cloned(),
        diagnostics,
    }))
}

fn reconstruct_request(
    headers: &HashMap<String, String>,
    payload: &[u8],
) -> Option<ObservableMessage> {
    let diagnostics = headers.get("diagnostics")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| RequestDiagnostics {
            sequence: 0,
            endpoint: String::new(),
            request_bytes: 0,
            received_at_ms: 0,
        });

    Some(ObservableMessage::Request(RequestMessage {
        id: MessageId::from(headers.get("msg-id")?.clone()),
        protocol_version: headers.get("protocol-version").cloned().unwrap_or_default(),
        timeline: headers.get("timeline-id").map(|s| TimelineId::from(s.clone())).unwrap_or_else(TimelineId::main),
        submission: SubmissionId::from(headers.get("submission-id")?.clone()),
        session: SessionId::from(headers.get("session-id")?.clone()),
        agent_id: AgentId::new(headers.get("agent-id")?),
        service: ServiceBackend::from_parts(
            ServiceType::from_str(headers.get("service")?)?,
            headers.get("backend")?,
        )?,
        operation: Operation::from_str(headers.get("operation")?)?,
        sequence: Sequence::from(
            headers.get("sequence")?.parse::<u32>().ok()?
        ),
        payload: RequestPayload::Legacy(payload.to_vec()),
        state: headers.get("state").cloned(),
        diagnostics,
    }))
}

fn reconstruct_response(
    headers: &HashMap<String, String>,
    payload: &[u8],
) -> Option<ObservableMessage> {
    let diagnostics = headers.get("diagnostics")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(ServiceDiagnostics::placeholder);

    Some(ObservableMessage::Response(ResponseMessage {
        id: MessageId::from(headers.get("msg-id")?.clone()),
        protocol_version: headers.get("protocol-version").cloned().unwrap_or_default(),
        timeline: headers.get("timeline-id").map(|s| TimelineId::from(s.clone())).unwrap_or_else(TimelineId::main),
        submission: SubmissionId::from(headers.get("submission-id")?.clone()),
        session: SessionId::from(headers.get("session-id")?.clone()),
        agent_id: AgentId::new(headers.get("agent-id")?),
        service: ServiceBackend::from_parts(
            ServiceType::from_str(headers.get("service")?)?,
            headers.get("backend")?,
        )?,
        operation: Operation::from_str(headers.get("operation")?)?,
        sequence: Sequence::from(
            headers.get("sequence")?.parse::<u32>().ok()?
        ),
        payload: ResponsePayload::Legacy(payload.to_vec()),
        correlation_id: MessageId::from(headers.get("correlation-id")?.clone()),
        state: headers.get("state").cloned(),
        diagnostics,
    }))
}

fn reconstruct_complete(
    headers: &HashMap<String, String>,
    payload: &[u8],
) -> Option<ObservableMessage> {
    let diagnostics = headers.get("diagnostics")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| ContainerDiagnostics::placeholder(0));

    Some(ObservableMessage::Complete(CompleteMessage {
        id: MessageId::from(headers.get("msg-id")?.clone()),
        protocol_version: headers.get("protocol-version").cloned().unwrap_or_default(),
        timeline: headers.get("timeline-id").map(|s| TimelineId::from(s.clone())).unwrap_or_else(TimelineId::main),
        submission: SubmissionId::from(headers.get("submission-id")?.clone()),
        session: SessionId::from(headers.get("session-id")?.clone()),
        agent_id: AgentId::new(headers.get("agent-id")?),
        harness: parse_harness(headers.get("harness")?)?,
        payload: payload.to_vec(),
        state: headers.get("state").cloned(),
        diagnostics,
    }))
}

fn reconstruct_delegate(
    headers: &HashMap<String, String>,
    payload: &[u8],
) -> Option<ObservableMessage> {
    let diagnostics = headers.get("diagnostics")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| DelegateDiagnostics {
            container: ContainerDiagnostics::placeholder(0),
        });

    Some(ObservableMessage::Delegate(DelegateMessage {
        id: MessageId::from(headers.get("msg-id")?.clone()),
        protocol_version: headers.get("protocol-version").cloned().unwrap_or_default(),
        timeline: headers.get("timeline-id").map(|s| TimelineId::from(s.clone())).unwrap_or_else(TimelineId::main),
        submission: SubmissionId::from(headers.get("submission-id")?.clone()),
        session: SessionId::from(headers.get("session-id")?.clone()),
        caller: AgentId::new(headers.get("caller-agent")?.clone()),
        target: AgentId::new(headers.get("target-agent")?.clone()),
        payload: payload.to_vec(),
        reply_subject: headers.get("reply-subject")?.clone(),
        state: headers.get("state").cloned(),
        diagnostics,
    }))
}

fn parse_harness(s: &str) -> Option<HarnessType> {
    match s {
        "cli" => Some(HarnessType::Cli),
        "web" => Some(HarnessType::Web),
        "api" => Some(HarnessType::Api),
        "whatsapp" => Some(HarnessType::Whatsapp),
        _ => None,
    }
}

fn parse_runtime(s: &str) -> Option<RuntimeType> {
    match s {
        "container" => Some(RuntimeType::Container),
        _ => None,
    }
}

// ============================================================================
// ObservableMessage → DagNode conversion helpers
// ============================================================================

/// Determine the `MessageType` for a reconstructed message.
pub fn observable_message_type(msg: &ObservableMessage) -> MessageType {
    match msg {
        ObservableMessage::Invoke(_) => MessageType::Invoke,
        ObservableMessage::Request(_) => MessageType::Request,
        ObservableMessage::Response(_) => MessageType::Response,
        ObservableMessage::Complete(_) => MessageType::Complete,
        ObservableMessage::Delegate(_) => MessageType::Delegate,
    }
}

/// Extract (from, to) routing pair from a reconstructed message.
pub fn observable_from_to(msg: &ObservableMessage) -> (String, String) {
    match msg {
        ObservableMessage::Invoke(m) => (
            m.harness.as_str().to_string(),
            m.agent_id.to_string(),
        ),
        ObservableMessage::Request(m) => (
            m.agent_id.to_string(),
            format!("{}.{}", m.service.service_type(), m.service.backend_str()),
        ),
        ObservableMessage::Response(m) => (
            format!("{}.{}", m.service.service_type(), m.service.backend_str()),
            m.agent_id.to_string(),
        ),
        ObservableMessage::Complete(m) => (
            m.agent_id.to_string(),
            m.harness.as_str().to_string(),
        ),
        ObservableMessage::Delegate(m) => (
            m.caller.to_string(),
            m.target.to_string(),
        ),
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
/// Stderr lives on `ContainerDiagnostics` — only Complete and Delegate carry it.
/// Returns empty for other message types.
pub fn extract_typed_stderr(msg: &ObservableMessage) -> Vec<u8> {
    match msg {
        ObservableMessage::Complete(m) => m.diagnostics.stderr.clone(),
        ObservableMessage::Delegate(m) => m.diagnostics.container.stderr.clone(),
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

/// Build a `DagNode` from a reconstructed `ObservableMessage` and Merkle chain state.
///
/// `parent_hash` is the hash of the previous node in the same session
/// (empty string for the first message).
pub fn build_dag_node(msg: &ObservableMessage, parent_hash: &str) -> DagNode {
    let message_type = observable_message_type(msg);
    let (from, to) = observable_from_to(msg);
    let diagnostics = serialize_diagnostics(msg);
    let stderr = extract_typed_stderr(msg);
    let state = observable_state(msg);
    let payload = msg.payload();
    let hash = hash_dag_node(payload, parent_hash, &message_type, &diagnostics);

    DagNode {
        hash,
        parent_hash: parent_hash.to_string(),
        message_type,
        from,
        to,
        session_id: msg.session().as_str().to_string(),
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
    use crate::domain::{DagStore, InferenceBackendType, InMemoryDagStore};

    // --- Header construction helpers ---

    fn invoke_headers() -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("msg-id".to_string(), "msg-001".to_string());
        h.insert("protocol-version".to_string(), "0.1.0".to_string());
        h.insert("timeline-id".to_string(), "1".to_string());
        h.insert("submission-id".to_string(), "sub-1".to_string());
        h.insert("session-id".to_string(), "sess-1".to_string());
        h.insert("harness".to_string(), "cli".to_string());
        h.insert("runtime".to_string(), "container".to_string());
        h.insert("agent-id".to_string(), "myagent".to_string());
        h
    }

    fn request_headers() -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("msg-id".to_string(), "msg-002".to_string());
        h.insert("protocol-version".to_string(), "0.1.0".to_string());
        h.insert("timeline-id".to_string(), "1".to_string());
        h.insert("submission-id".to_string(), "sub-1".to_string());
        h.insert("session-id".to_string(), "sess-1".to_string());
        h.insert("agent-id".to_string(), "myagent".to_string());
        h.insert("service".to_string(), "infer".to_string());
        h.insert("backend".to_string(), "ollama".to_string());
        h.insert("operation".to_string(), "run".to_string());
        h.insert("sequence".to_string(), "1".to_string());
        h
    }

    fn response_headers() -> HashMap<String, String> {
        let mut h = request_headers();
        h.insert("msg-id".to_string(), "msg-003".to_string());
        h.insert("correlation-id".to_string(), "msg-002".to_string());
        h
    }

    fn complete_headers() -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("msg-id".to_string(), "msg-004".to_string());
        h.insert("protocol-version".to_string(), "0.1.0".to_string());
        h.insert("timeline-id".to_string(), "1".to_string());
        h.insert("submission-id".to_string(), "sub-1".to_string());
        h.insert("session-id".to_string(), "sess-1".to_string());
        h.insert("agent-id".to_string(), "myagent".to_string());
        h.insert("harness".to_string(), "cli".to_string());
        h
    }

    fn delegate_headers() -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("msg-id".to_string(), "msg-005".to_string());
        h.insert("protocol-version".to_string(), "0.1.0".to_string());
        h.insert("timeline-id".to_string(), "1".to_string());
        h.insert("submission-id".to_string(), "sub-1".to_string());
        h.insert("session-id".to_string(), "sess-1".to_string());
        h.insert("caller-agent".to_string(), "coordinator".to_string());
        h.insert("target-agent".to_string(), "summarizer".to_string());
        h.insert("reply-subject".to_string(), "vlinder.sub-1.delegate-reply".to_string());
        h
    }

    // --- reconstruct_observable_message tests ---

    #[test]
    fn reconstruct_invoke() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.invoke.cli.container.myagent",
            &invoke_headers(),
            b"invoke-payload",
        ).unwrap();

        assert!(matches!(msg, ObservableMessage::Invoke(_)));
        if let ObservableMessage::Invoke(m) = &msg {
            assert_eq!(m.harness, HarnessType::Cli);
            assert_eq!(m.runtime, RuntimeType::Container);
            assert_eq!(m.agent_id.as_str(), "myagent");
            assert_eq!(m.payload, b"invoke-payload");
            assert_eq!(m.session.as_str(), "sess-1");
            assert_eq!(m.submission.as_str(), "sub-1");
        }
    }

    #[test]
    fn reconstruct_request_msg() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.req.myagent.infer.ollama.run.1",
            &request_headers(),
            b"request-payload",
        ).unwrap();

        assert!(matches!(msg, ObservableMessage::Request(_)));
        if let ObservableMessage::Request(m) = &msg {
            assert_eq!(m.service, ServiceBackend::Infer(InferenceBackendType::Ollama));
            assert_eq!(m.operation, Operation::Run);
            assert_eq!(m.sequence.as_u32(), 1);
            assert_eq!(m.payload.legacy_bytes(), b"request-payload");
        }
    }

    #[test]
    fn reconstruct_response_msg() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.res.infer.ollama.myagent.run.1",
            &response_headers(),
            b"response-payload",
        ).unwrap();

        assert!(matches!(msg, ObservableMessage::Response(_)));
        if let ObservableMessage::Response(m) = &msg {
            assert_eq!(m.service, ServiceBackend::Infer(InferenceBackendType::Ollama));
            assert_eq!(m.correlation_id.as_str(), "msg-002");
            assert_eq!(m.payload.legacy_bytes(), b"response-payload");
        }
    }

    #[test]
    fn reconstruct_complete_msg() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.complete.myagent.cli",
            &complete_headers(),
            b"complete-payload",
        ).unwrap();

        assert!(matches!(msg, ObservableMessage::Complete(_)));
        if let ObservableMessage::Complete(m) = &msg {
            assert_eq!(m.harness, HarnessType::Cli);
            assert_eq!(m.payload, b"complete-payload");
        }
    }

    #[test]
    fn reconstruct_delegate_msg() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.delegate.coordinator.summarizer",
            &delegate_headers(),
            b"delegate-payload",
        ).unwrap();

        assert!(matches!(msg, ObservableMessage::Delegate(_)));
        if let ObservableMessage::Delegate(m) = &msg {
            assert_eq!(m.caller, AgentId::new("coordinator"));
            assert_eq!(m.target, AgentId::new("summarizer"));
            assert_eq!(m.reply_subject, "vlinder.sub-1.delegate-reply");
            assert_eq!(m.payload, b"delegate-payload");
        }
    }

    #[test]
    fn unknown_message_type_returns_none() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.foobar.something",
            &invoke_headers(),
            b"payload",
        );
        assert!(msg.is_none());
    }

    #[test]
    fn missing_required_header_returns_none() {
        // Missing session-id
        let mut h = invoke_headers();
        h.remove("session-id");
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.invoke.cli.container.agent",
            &h,
            b"payload",
        );
        assert!(msg.is_none());
    }

    #[test]
    fn state_header_preserved() {
        let mut h = invoke_headers();
        h.insert("state".to_string(), "abc123state".to_string());
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.invoke.cli.container.myagent",
            &h,
            b"payload",
        ).unwrap();

        if let ObservableMessage::Invoke(m) = &msg {
            assert_eq!(m.state, Some("abc123state".to_string()));
        } else {
            panic!("expected Invoke");
        }
    }

    #[test]
    fn missing_state_gives_none() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.invoke.cli.container.myagent",
            &invoke_headers(),
            b"payload",
        ).unwrap();

        if let ObservableMessage::Invoke(m) = &msg {
            assert_eq!(m.state, None);
        } else {
            panic!("expected Invoke");
        }
    }

    // --- ObservableMessage → DagNode conversion tests ---

    #[test]
    fn build_dag_node_from_invoke() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.invoke.cli.container.myagent",
            &invoke_headers(),
            b"invoke-payload",
        ).unwrap();

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
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.req.myagent.infer.ollama.run.1",
            &request_headers(),
            b"request-payload",
        ).unwrap();

        let node = build_dag_node(&msg, "parent-abc");
        assert_eq!(node.message_type, MessageType::Request);
        assert_eq!(node.from, "myagent");
        assert_eq!(node.to, "infer.ollama");
        assert_eq!(node.parent_hash, "parent-abc");
    }

    #[test]
    fn build_dag_node_from_response() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.res.infer.ollama.myagent.run.1",
            &response_headers(),
            b"response-payload",
        ).unwrap();

        let node = build_dag_node(&msg, "");
        assert_eq!(node.message_type, MessageType::Response);
        assert_eq!(node.from, "infer.ollama");
        assert_eq!(node.to, "myagent");
    }

    #[test]
    fn build_dag_node_from_delegate() {
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.delegate.coordinator.summarizer",
            &delegate_headers(),
            b"delegate-payload",
        ).unwrap();

        let node = build_dag_node(&msg, "");
        assert_eq!(node.message_type, MessageType::Delegate);
        assert_eq!(node.from, "coordinator");
        assert_eq!(node.to, "summarizer");
    }

    #[test]
    fn build_dag_node_state_preserved() {
        let mut h = complete_headers();
        h.insert("state".to_string(), "state-hash".to_string());
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.complete.myagent.cli",
            &h,
            b"done",
        ).unwrap();

        let node = build_dag_node(&msg, "");
        assert_eq!(node.state, Some("state-hash".to_string()));
    }

    // --- Integration: reconstruct → DagNode → SQLite round-trip ---

    fn test_store() -> InMemoryDagStore {
        InMemoryDagStore::new()
    }

    #[test]
    fn reconstruct_and_store_invoke() {
        let store = test_store();
        let msg = reconstruct_observable_message(
            "vlinder.1.sub-1.invoke.cli.container.myagent",
            &invoke_headers(),
            b"invoke-payload",
        ).unwrap();

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

        let msg1 = reconstruct_observable_message(
            "vlinder.1.sub-1.invoke.cli.container.agent-a",
            &invoke_headers(),
            b"first",
        ).unwrap();
        let node1 = build_dag_node(&msg1, &last_hash);
        last_hash = node1.hash.clone();
        store.insert_node(&node1).unwrap();

        let msg2 = reconstruct_observable_message(
            "vlinder.1.sub-1.req.myagent.infer.ollama.run.1",
            &request_headers(),
            b"second",
        ).unwrap();
        let node2 = build_dag_node(&msg2, &last_hash);
        last_hash = node2.hash.clone();
        store.insert_node(&node2).unwrap();

        let msg3 = reconstruct_observable_message(
            "vlinder.1.sub-1.res.infer.ollama.myagent.run.1",
            &response_headers(),
            b"third",
        ).unwrap();
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

        let msg1 = reconstruct_observable_message(
            "vlinder.1.sub-1.invoke.cli.container.agent-a",
            &invoke_headers(),
            b"sess1-first",
        ).unwrap();
        let node1 = build_dag_node(&msg1, "");
        store.insert_node(&node1).unwrap();

        let mut h2 = invoke_headers();
        h2.insert("session-id".to_string(), "sess-2".to_string());
        h2.insert("msg-id".to_string(), "msg-099".to_string());
        let msg2 = reconstruct_observable_message(
            "vlinder.1.sub-2.invoke.cli.container.agent-b",
            &h2,
            b"sess2-first",
        ).unwrap();
        let node2 = build_dag_node(&msg2, "");
        store.insert_node(&node2).unwrap();

        let msg3 = reconstruct_observable_message(
            "vlinder.1.sub-1.complete.myagent.cli",
            &complete_headers(),
            b"sess1-second",
        ).unwrap();
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
