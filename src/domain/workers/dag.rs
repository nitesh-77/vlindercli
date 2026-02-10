//! DagCaptureWorker — pairs invoke/complete messages and writes DAG nodes (ADR 062).
//!
//! Subscribes to `vlinder.>` via its own JetStream consumer and passively
//! observes all traffic. When it sees an invoke followed by its matching
//! complete, it computes a content hash, chains to the previous node in
//! that session, and writes a DAG node.
//!
//! No agent cooperation required — the platform captures everything.

use std::collections::HashMap;

use crate::storage::dag_store::{DagNode, DagStore, hash_dag_node};

/// Holds the invoke payload while waiting for the matching complete.
struct PendingInvoke {
    session_id: String,
    agent: String,
    payload_in: Vec<u8>,
}

/// Worker that captures invoke/complete pairs and writes DAG nodes.
pub struct DagCaptureWorker {
    store: Box<dyn DagStore>,
    /// Pending invokes keyed by (submission_id, agent_name).
    pending: HashMap<(String, String), PendingInvoke>,
    /// Last node hash per session_id — for Merkle chaining.
    last_node: HashMap<String, String>,
}

impl DagCaptureWorker {
    pub fn new(store: Box<dyn DagStore>) -> Self {
        Self {
            store,
            pending: HashMap::new(),
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
        let msg_type = segments[2];

        match msg_type {
            "invoke" => self.handle_invoke(submission_id, headers, payload),
            "complete" => self.handle_complete(submission_id, headers, payload),
            _ => {} // ignore req/res/delegate — not agent boundary messages
        }
    }

    fn handle_invoke(
        &mut self,
        submission_id: &str,
        headers: &HashMap<String, String>,
        payload: &[u8],
    ) {
        let session_id = match headers.get("session-id") {
            Some(s) => s.clone(),
            None => return,
        };
        let agent = match headers.get("agent-id") {
            Some(s) => s.clone(),
            None => return,
        };

        let key = (submission_id.to_string(), agent.clone());
        self.pending.insert(key, PendingInvoke {
            session_id,
            agent,
            payload_in: payload.to_vec(),
        });
    }

    fn handle_complete(
        &mut self,
        submission_id: &str,
        headers: &HashMap<String, String>,
        payload: &[u8],
    ) {
        let agent = match headers.get("agent-id") {
            Some(s) => s.clone(),
            None => return,
        };

        let key = (submission_id.to_string(), agent);
        let pending = match self.pending.remove(&key) {
            Some(p) => p,
            None => return, // unmatched complete — no crash, just skip
        };

        let parent_hash = self.last_node
            .get(&pending.session_id)
            .cloned()
            .unwrap_or_default();

        let hash = hash_dag_node(&pending.payload_in, payload, &parent_hash);

        let now = chrono_now();
        let node = DagNode {
            hash: hash.clone(),
            parent_hash,
            agent: pending.agent,
            session_id: pending.session_id.clone(),
            payload_in: pending.payload_in,
            payload_out: payload.to_vec(),
            created_at: now,
        };

        if let Err(e) = self.store.insert_node(&node) {
            tracing::error!(error = %e, hash = %hash, "Failed to write DAG node");
            return;
        }

        // Update chain pointer for this session
        self.last_node.insert(pending.session_id, hash);
    }
}

/// Simple ISO-8601 timestamp without pulling in chrono.
fn chrono_now() -> String {
    // Use std::time for a simple UTC timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // Format as seconds — good enough for ordering. Full ISO-8601 would need chrono.
    format!("{}", now.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::dag_store::{SqliteDagStore, hash_dag_node};

    fn test_store() -> Box<SqliteDagStore> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Box::new(SqliteDagStore::open(tmp.path()).unwrap())
    }

    fn invoke_headers(session_id: &str, agent_id: &str) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("session-id".to_string(), session_id.to_string());
        h.insert("agent-id".to_string(), agent_id.to_string());
        h
    }

    fn complete_headers(agent_id: &str) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("agent-id".to_string(), agent_id.to_string());
        h
    }

    #[test]
    fn invoke_then_complete_writes_dag_node() {
        let store = test_store();
        // Keep a reference for querying
        let store_ref = SqliteDagStore::open(std::path::Path::new("/dev/null")).ok();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let query_store = SqliteDagStore::open(tmp.path()).unwrap();
        let worker_store = Box::new(SqliteDagStore::open(tmp.path()).unwrap());

        let mut worker = DagCaptureWorker::new(worker_store);

        let invoke_subject = "vlinder.sub-1.invoke.cli.container.myagent";
        let complete_subject = "vlinder.sub-1.complete.myagent.cli";

        worker.process_message(
            invoke_subject,
            &invoke_headers("sess-1", "agent-a"),
            b"invoke-payload",
        );
        worker.process_message(
            complete_subject,
            &complete_headers("agent-a"),
            b"complete-payload",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].agent, "agent-a");
        assert_eq!(nodes[0].session_id, "sess-1");
        assert_eq!(nodes[0].payload_in, b"invoke-payload");
        assert_eq!(nodes[0].payload_out, b"complete-payload");
        assert_eq!(nodes[0].parent_hash, "");
        assert_eq!(
            nodes[0].hash,
            hash_dag_node(b"invoke-payload", b"complete-payload", "")
        );
        drop(store_ref);
        drop(store);
    }

    #[test]
    fn second_node_chains_to_first() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let query_store = SqliteDagStore::open(tmp.path()).unwrap();
        let worker_store = Box::new(SqliteDagStore::open(tmp.path()).unwrap());

        let mut worker = DagCaptureWorker::new(worker_store);

        // First invoke/complete
        worker.process_message(
            "vlinder.sub-1.invoke.cli.container.a",
            &invoke_headers("sess-1", "agent-a"),
            b"in-1",
        );
        worker.process_message(
            "vlinder.sub-1.complete.a.cli",
            &complete_headers("agent-a"),
            b"out-1",
        );

        // Second invoke/complete in same session
        worker.process_message(
            "vlinder.sub-2.invoke.cli.container.a",
            &invoke_headers("sess-1", "agent-a"),
            b"in-2",
        );
        worker.process_message(
            "vlinder.sub-2.complete.a.cli",
            &complete_headers("agent-a"),
            b"out-2",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 2);

        // Second node's parent is first node's hash
        let first_hash = hash_dag_node(b"in-1", b"out-1", "");
        assert_eq!(nodes[1].parent_hash, first_hash);

        // Second node's hash includes parent
        let second_hash = hash_dag_node(b"in-2", b"out-2", &first_hash);
        assert_eq!(nodes[1].hash, second_hash);
    }

    #[test]
    fn unmatched_complete_does_not_crash() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let query_store = SqliteDagStore::open(tmp.path()).unwrap();
        let worker_store = Box::new(SqliteDagStore::open(tmp.path()).unwrap());

        let mut worker = DagCaptureWorker::new(worker_store);

        // Complete without prior invoke — should be silently ignored
        worker.process_message(
            "vlinder.sub-1.complete.a.cli",
            &complete_headers("agent-a"),
            b"orphan-output",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn multiple_agents_in_same_session() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let query_store = SqliteDagStore::open(tmp.path()).unwrap();
        let worker_store = Box::new(SqliteDagStore::open(tmp.path()).unwrap());

        let mut worker = DagCaptureWorker::new(worker_store);

        // Agent A invoke/complete
        worker.process_message(
            "vlinder.sub-1.invoke.cli.container.a",
            &invoke_headers("sess-1", "agent-a"),
            b"a-in",
        );
        worker.process_message(
            "vlinder.sub-1.complete.a.cli",
            &complete_headers("agent-a"),
            b"a-out",
        );

        // Agent B invoke/complete in same session (different submission)
        worker.process_message(
            "vlinder.sub-2.invoke.cli.container.b",
            &invoke_headers("sess-1", "agent-b"),
            b"b-in",
        );
        worker.process_message(
            "vlinder.sub-2.complete.b.cli",
            &complete_headers("agent-b"),
            b"b-out",
        );

        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 2);

        // Both should be present with different agents
        let agents: Vec<&str> = nodes.iter().map(|n| n.agent.as_str()).collect();
        assert!(agents.contains(&"agent-a"));
        assert!(agents.contains(&"agent-b"));

        // Second node chains to first (same session)
        assert_eq!(nodes[1].parent_hash, nodes[0].hash);
    }

    #[test]
    fn ignores_non_boundary_messages() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let query_store = SqliteDagStore::open(tmp.path()).unwrap();
        let worker_store = Box::new(SqliteDagStore::open(tmp.path()).unwrap());

        let mut worker = DagCaptureWorker::new(worker_store);

        // Request and response messages should be ignored
        worker.process_message(
            "vlinder.sub-1.req.agent.infer.ollama.run.1",
            &HashMap::new(),
            b"request-payload",
        );
        worker.process_message(
            "vlinder.sub-1.res.infer.ollama.agent.run.1",
            &HashMap::new(),
            b"response-payload",
        );

        // No nodes should be written
        let nodes = query_store.get_session_nodes("sess-1").unwrap();
        assert!(nodes.is_empty());
    }
}
