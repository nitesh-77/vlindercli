//! DagCaptureWorker — stateless NATS dispatcher that fans out to DAG workers (ADR 065, 067).
//!
//! Subscribes to `vlinder.>` via its own JetStream consumer and captures
//! every message as an independent DAG node. No pairing, no buffering.
//!
//! Message arrives → parse subject → build node → dispatch to workers. Done.
//! The only state is `last_node` for Merkle chaining per session.
//!
//! Workers are pluggable projections (ADR 065). Each implements `DagWorker`
//! and receives every node. SQLite for queries, git for time-travel — both
//! are just projections of the same NATS event stream.

use std::collections::HashMap;

use crate::storage::dag_store::{DagNode, DagStore, MessageType, hash_dag_node};

/// A projection that receives DAG nodes (ADR 065).
///
/// Any backend that wants to observe the NATS message stream implements this.
/// `SqliteDagWorker` writes to SQLite for queries. `GitDagWorker` (future)
/// writes git commits for time-travel debugging. Both receive every node.
pub trait DagWorker: Send {
    /// Called for each node the dispatcher builds from a NATS message.
    fn on_message(&mut self, node: &DagNode);
}

/// DAG worker that writes nodes to a `DagStore` (SQLite).
pub struct SqliteDagWorker {
    store: Box<dyn DagStore>,
}

impl SqliteDagWorker {
    pub fn new(store: Box<dyn DagStore>) -> Self {
        Self { store }
    }
}

impl DagWorker for SqliteDagWorker {
    fn on_message(&mut self, node: &DagNode) {
        if let Err(e) = self.store.insert_node(node) {
            tracing::error!(error = %e, hash = %node.hash, "Failed to write DAG node");
        }
    }
}

/// Dispatcher that parses NATS messages and fans out to DagWorkers.
pub struct DagCaptureWorker {
    workers: Vec<Box<dyn DagWorker>>,
    /// Last node hash per session_id — for Merkle chaining.
    last_node: HashMap<String, String>,
}

impl DagCaptureWorker {
    pub fn new(workers: Vec<Box<dyn DagWorker>>) -> Self {
        Self {
            workers,
            last_node: HashMap::new(),
        }
    }

    /// Process a single NATS message.
    ///
    /// `subject` is the NATS subject (e.g. `vlinder.sub123.invoke.cli.container.myagent`).
    /// `headers` contains NATS message headers as a simple map.
    /// `payload` is the raw message body.
    pub fn process_message(
        &mut self,
        subject: &str,
        headers: &HashMap<String, String>,
        payload: &[u8],
    ) {
        let segments: Vec<&str> = subject.split('.').collect();

        // Minimum: vlinder.<submission>.<type>...
        if segments.len() < 3 || segments[0] != "vlinder" {
            return;
        }

        let submission_id = segments[1];
        let msg_type_str = segments[2];

        let message_type = match MessageType::from_str(msg_type_str) {
            Some(mt) => mt,
            None => return, // Unknown message type — skip
        };

        let session_id = match headers.get("session-id") {
            Some(s) => s.clone(),
            None => return,
        };

        // Parse from/to from the remaining subject segments.
        // Actual NATS subject patterns (from queue/nats.rs):
        //   invoke:   vlinder.<sub>.invoke.<harness>.<runtime>.<agent>
        //   complete: vlinder.<sub>.complete.<agent>.<harness>
        //   req:      vlinder.<sub>.req.<agent>.<service>.<backend>.<op>.<seq>
        //   res:      vlinder.<sub>.res.<service>.<backend>.<agent>.<op>.<seq>
        //   delegate: vlinder.<sub>.delegate.<caller>.<target>
        let (from, to) = parse_from_to(message_type, &segments[3..]);

        let parent_hash = self.last_node
            .get(&session_id)
            .cloned()
            .unwrap_or_default();

        // Extract diagnostics from NATS headers (ADR 071).
        let diagnostics = headers.get("diagnostics")
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();

        // Extract stderr from container diagnostics (Complete/Delegate only, ADR 071).
        let stderr = extract_stderr(&diagnostics);

        let hash = hash_dag_node(payload, &parent_hash, &message_type, &diagnostics);

        let now = chrono_now();
        let node = DagNode {
            hash: hash.clone(),
            parent_hash,
            message_type,
            from,
            to,
            session_id: session_id.clone(),
            submission_id: submission_id.to_string(),
            payload: payload.to_vec(),
            diagnostics,
            stderr,
            created_at: now,
        };

        for worker in &mut self.workers {
            worker.on_message(&node);
        }

        // Update chain pointer for this session
        self.last_node.insert(session_id, hash);
    }
}

/// Parse from/to from subject segments after the message type.
fn parse_from_to(message_type: MessageType, rest: &[&str]) -> (String, String) {
    match message_type {
        // invoke: <harness>.<runtime>.<agent>
        MessageType::Invoke => {
            let from = rest.first().copied().unwrap_or("unknown");
            let to = rest.last().copied().unwrap_or("unknown");
            (from.to_string(), to.to_string())
        }
        // complete: <agent>.<harness>
        MessageType::Complete => {
            let from = rest.first().copied().unwrap_or("unknown");
            let to = rest.last().copied().unwrap_or("unknown");
            (from.to_string(), to.to_string())
        }
        // req: <agent>.<service>.<backend>.<op>.<seq>
        MessageType::Request => {
            let from = rest.first().copied().unwrap_or("unknown");
            let to = if rest.len() >= 2 {
                format!("{}.{}", rest[1], rest.get(2).copied().unwrap_or(""))
            } else {
                "unknown".to_string()
            };
            (from.to_string(), to)
        }
        // res: <service>.<backend>.<agent>.<op>.<seq>
        MessageType::Response => {
            let from = if rest.len() >= 2 {
                format!("{}.{}", rest[0], rest[1])
            } else {
                rest.first().copied().unwrap_or("unknown").to_string()
            };
            let to = rest.get(2).copied().unwrap_or("unknown");
            (from, to.to_string())
        }
        // delegate: <caller>.<target>
        MessageType::Delegate => {
            let from = rest.first().copied().unwrap_or("unknown");
            let to = rest.get(1).copied().unwrap_or("unknown");
            (from.to_string(), to.to_string())
        }
    }
}

/// Extract stderr bytes from the diagnostics JSON (ADR 071).
///
/// ContainerDiagnostics and DelegateDiagnostics both carry a `stderr` field
/// (as a byte array serialized to JSON). For other message types, returns empty.
fn extract_stderr(diagnostics: &[u8]) -> Vec<u8> {
    if diagnostics.is_empty() {
        return Vec::new();
    }
    // Try parsing as ContainerDiagnostics or DelegateDiagnostics
    if let Ok(map) = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(diagnostics) {
        // Direct stderr field (ContainerDiagnostics)
        if let Some(serde_json::Value::Array(arr)) = map.get("stderr") {
            return arr.iter()
                .filter_map(|v| v.as_u64().map(|b| b as u8))
                .collect();
        }
        // Nested in container field (DelegateDiagnostics)
        if let Some(serde_json::Value::Object(container)) = map.get("container") {
            if let Some(serde_json::Value::Array(arr)) = container.get("stderr") {
                return arr.iter()
                    .filter_map(|v| v.as_u64().map(|b| b as u8))
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Simple timestamp without pulling in chrono.
fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::dag_store::SqliteDagStore;

    fn test_store_pair() -> (Box<SqliteDagStore>, SqliteDagStore) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let worker_store = Box::new(SqliteDagStore::open(tmp.path()).unwrap());
        let query_store = SqliteDagStore::open(tmp.path()).unwrap();
        (worker_store, query_store)
    }

    fn make_dispatcher(store: Box<SqliteDagStore>) -> DagCaptureWorker {
        let sqlite_worker = SqliteDagWorker::new(store);
        DagCaptureWorker::new(vec![Box::new(sqlite_worker)])
    }

    fn headers(session_id: &str) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("session-id".to_string(), session_id.to_string());
        h
    }

    #[test]
    fn invoke_writes_dag_node() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.invoke.cli.container.myagent",
            &headers("sess-1"),
            b"invoke-payload",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Invoke);
        assert_eq!(nodes[0].from, "cli");
        assert_eq!(nodes[0].to, "myagent");
        assert_eq!(nodes[0].submission_id, "sub-1");
        assert_eq!(nodes[0].payload, b"invoke-payload");
        assert_eq!(nodes[0].parent_hash, "");
    }

    #[test]
    fn complete_writes_dag_node() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.complete.myagent.cli",
            &headers("sess-1"),
            b"complete-payload",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Complete);
        assert_eq!(nodes[0].from, "myagent");
        assert_eq!(nodes[0].to, "cli");
    }

    #[test]
    fn request_writes_dag_node() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.req.myagent.infer.ollama.run.1",
            &headers("sess-1"),
            b"request-payload",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Request);
        assert_eq!(nodes[0].from, "myagent");
        assert_eq!(nodes[0].to, "infer.ollama");
    }

    #[test]
    fn response_writes_dag_node() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.res.infer.ollama.myagent.run.1",
            &headers("sess-1"),
            b"response-payload",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Response);
        assert_eq!(nodes[0].from, "infer.ollama");
        assert_eq!(nodes[0].to, "myagent");
    }

    #[test]
    fn delegate_writes_dag_node() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.delegate.coordinator.summarizer",
            &headers("sess-1"),
            b"delegate-payload",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].message_type, MessageType::Delegate);
        assert_eq!(nodes[0].from, "coordinator");
        assert_eq!(nodes[0].to, "summarizer");
    }

    #[test]
    fn messages_chain_in_session() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.invoke.cli.container.agent-a",
            &headers("sess-1"),
            b"first",
        );
        dispatcher.process_message(
            "vlinder.sub-1.req.agent-a.infer.ollama.run.1",
            &headers("sess-1"),
            b"second",
        );
        dispatcher.process_message(
            "vlinder.sub-1.res.infer.ollama.agent-a.run.1",
            &headers("sess-1"),
            b"third",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 3);

        // First node has empty parent
        assert_eq!(nodes[0].parent_hash, "");
        // Second chains to first
        assert_eq!(nodes[1].parent_hash, nodes[0].hash);
        // Third chains to second
        assert_eq!(nodes[2].parent_hash, nodes[1].hash);
    }

    #[test]
    fn unknown_message_type_is_ignored() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.foobar.something",
            &headers("sess-1"),
            b"payload",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn missing_session_header_is_ignored() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.invoke.cli.container.agent",
            &HashMap::new(),
            b"payload",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn different_sessions_chain_independently() {
        let (worker_store, query_store) = test_store_pair();
        let mut dispatcher = make_dispatcher(worker_store);

        dispatcher.process_message(
            "vlinder.sub-1.invoke.cli.container.a",
            &headers("sess-1"),
            b"sess1-first",
        );
        dispatcher.process_message(
            "vlinder.sub-2.invoke.cli.container.b",
            &headers("sess-2"),
            b"sess2-first",
        );
        dispatcher.process_message(
            "vlinder.sub-1.complete.a.cli",
            &headers("sess-1"),
            b"sess1-second",
        );

        let sess1 = query_store.get_session_nodes("sess-1").unwrap();
        let sess2 = query_store.get_session_nodes("sess-2").unwrap();

        assert_eq!(sess1.len(), 2);
        assert_eq!(sess2.len(), 1);

        // sess-1's second node chains to sess-1's first, not sess-2's
        assert_eq!(sess1[1].parent_hash, sess1[0].hash);
        // sess-2's first node has empty parent
        assert_eq!(sess2[0].parent_hash, "");
    }

    #[test]
    fn fan_out_to_multiple_workers() {
        // Two independent SQLite stores, both receiving every node.
        let tmp1 = tempfile::NamedTempFile::new().unwrap();
        let tmp2 = tempfile::NamedTempFile::new().unwrap();

        let store1 = Box::new(SqliteDagStore::open(tmp1.path()).unwrap());
        let store2 = Box::new(SqliteDagStore::open(tmp2.path()).unwrap());

        let query1 = SqliteDagStore::open(tmp1.path()).unwrap();
        let query2 = SqliteDagStore::open(tmp2.path()).unwrap();

        let worker1 = SqliteDagWorker::new(store1);
        let worker2 = SqliteDagWorker::new(store2);

        let mut dispatcher = DagCaptureWorker::new(vec![
            Box::new(worker1),
            Box::new(worker2),
        ]);

        dispatcher.process_message(
            "vlinder.sub-1.invoke.cli.container.agent-a",
            &headers("sess-1"),
            b"hello",
        );

        let nodes1 = query1.get_session_nodes("sess-1").unwrap();
        let nodes2 = query2.get_session_nodes("sess-1").unwrap();

        assert_eq!(nodes1.len(), 1);
        assert_eq!(nodes2.len(), 1);
        assert_eq!(nodes1[0].hash, nodes2[0].hash);
    }
}
