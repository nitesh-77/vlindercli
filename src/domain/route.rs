//! Route — the full chain of conversations a request triggers (ADR 063).
//!
//! A Route is every stop between user input and user output. Each Stop is
//! one message in the protocol trace. A single-agent session is a Route
//! with a few stops. A fleet produces a Route with many stops.
//!
//! Route is a domain type, not a presentation concern. Consumers format;
//! they don't interpret.

use chrono::{DateTime, Utc};

use crate::storage::dag_store::{DagNode, MessageType};

/// The full chain of conversations in a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub session_id: String,
    pub stops: Vec<Stop>,
}

/// One message in the protocol trace — a stop in the route.
///
/// Stripped of Merkle chain details (parent_hash). Keeps what
/// describes the interaction: who, what, when.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub hash: String,
    pub message_type: MessageType,
    pub from: String,
    pub to: String,
    pub payload: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

impl Route {
    /// Construct a Route from ordered DAG nodes.
    ///
    /// The caller queries the store and passes the result. No storage
    /// dependency in the domain type. Construction is a projection:
    /// strip Merkle chain details, keep what describes each stop.
    pub fn from_dag_nodes(session_id: String, nodes: Vec<DagNode>) -> Self {
        let stops = nodes.into_iter().map(|node| Stop {
            hash: node.hash,
            message_type: node.message_type,
            from: node.from,
            to: node.to,
            payload: node.payload,
            created_at: node.created_at,
        }).collect();

        Self { session_id, stops }
    }

    /// Ordered stops in the route.
    pub fn stops(&self) -> &[Stop] {
        &self.stops
    }

    /// Number of stops in the route.
    pub fn stop_count(&self) -> usize {
        self.stops.len()
    }

    /// Unique agents in invocation order.
    ///
    /// An "agent" is any entity that appears in the `from` or `to` field.
    /// Returns them in the order they first appear.
    pub fn agents(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for stop in &self.stops {
            if !seen.contains(&stop.from.as_str()) {
                seen.push(&stop.from);
            }
            if !seen.contains(&stop.to.as_str()) {
                seen.push(&stop.to);
            }
        }
        seen
    }

    /// Elapsed seconds from first to last stop.
    ///
    /// Returns None if the route has fewer than two stops or if timestamps
    /// can't be parsed as unix seconds.
    pub fn duration_secs(&self) -> Option<f64> {
        if self.stops.len() < 2 {
            return None;
        }
        let first = self.stops.first()?.created_at;
        let last = self.stops.last()?.created_at;
        Some((last - first).num_seconds() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::dag_store::hash_dag_node;

    fn make_node(
        payload: &[u8],
        parent_hash: &str,
        message_type: MessageType,
        from: &str,
        to: &str,
        epoch_secs: i64,
    ) -> DagNode {
        DagNode {
            hash: hash_dag_node(payload, parent_hash, &message_type, b""),
            parent_hash: parent_hash.to_string(),
            message_type,
            from: from.to_string(),
            to: to.to_string(),
            session_id: "sess-1".to_string(),
            submission_id: "sub-1".to_string(),
            payload: payload.to_vec(),
            diagnostics: Vec::new(),
            stderr: Vec::new(),
            created_at: DateTime::from_timestamp(epoch_secs, 0).unwrap(),
        }
    }

    #[test]
    fn empty_route() {
        let route = Route::from_dag_nodes("sess-1".to_string(), vec![]);
        assert_eq!(route.stop_count(), 0);
        assert_eq!(route.agents().len(), 0);
        assert_eq!(route.duration_secs(), None);
    }

    #[test]
    fn single_message_route() {
        let node = make_node(b"hello", "", MessageType::Invoke, "cli", "agent-a", 1000);
        let route = Route::from_dag_nodes("sess-1".to_string(), vec![node]);

        assert_eq!(route.stop_count(), 1);
        assert_eq!(route.stops()[0].message_type, MessageType::Invoke);
        assert_eq!(route.stops()[0].from, "cli");
        assert_eq!(route.stops()[0].to, "agent-a");
        assert_eq!(route.duration_secs(), None); // need 2+ stops
    }

    #[test]
    fn strips_merkle_chain_details() {
        let node = make_node(b"data", "parent-abc", MessageType::Request, "a", "b", 1000);
        let route = Route::from_dag_nodes("sess-1".to_string(), vec![node.clone()]);

        // Stop has the hash but NOT parent_hash or submission_id
        assert_eq!(route.stops()[0].hash, node.hash);
        assert_eq!(route.stops()[0].payload, b"data");
    }

    #[test]
    fn agents_in_invocation_order() {
        let n1 = make_node(b"1", "", MessageType::Invoke, "cli", "support-agent", 1000);
        let n2 = make_node(b"2", &n1.hash, MessageType::Request, "support-agent", "infer.ollama", 1001);
        let n3 = make_node(b"3", &n2.hash, MessageType::Response, "infer.ollama", "support-agent", 1002);
        let n4 = make_node(b"4", &n3.hash, MessageType::Delegate, "support-agent", "log-analyst", 1003);
        let n5 = make_node(b"5", &n4.hash, MessageType::Complete, "support-agent", "cli", 1004);

        let route = Route::from_dag_nodes("sess-1".to_string(), vec![n1, n2, n3, n4, n5]);

        let agents = route.agents();
        assert_eq!(agents, vec!["cli", "support-agent", "infer.ollama", "log-analyst"]);
    }

    #[test]
    fn agents_deduplicates() {
        let n1 = make_node(b"1", "", MessageType::Invoke, "cli", "agent-a", 1000);
        let n2 = make_node(b"2", &n1.hash, MessageType::Complete, "agent-a", "cli", 1001);

        let route = Route::from_dag_nodes("sess-1".to_string(), vec![n1, n2]);

        // cli and agent-a appear twice each, but agents() deduplicates
        let agents = route.agents();
        assert_eq!(agents, vec!["cli", "agent-a"]);
    }

    #[test]
    fn duration_secs() {
        let n1 = make_node(b"1", "", MessageType::Invoke, "cli", "a", 1000);
        let n2 = make_node(b"2", &n1.hash, MessageType::Request, "a", "b", 1002);
        let n3 = make_node(b"3", &n2.hash, MessageType::Complete, "a", "cli", 1005);

        let route = Route::from_dag_nodes("sess-1".to_string(), vec![n1, n2, n3]);
        assert_eq!(route.duration_secs(), Some(5.0));
    }

    #[test]
    fn fleet_route_shows_all_stops() {
        // Simulate: user → support-agent → (infer) → delegate to log-analyst → (infer) → complete
        let n1 = make_node(b"q",  "",       MessageType::Invoke,   "cli",           "support-agent", 100);
        let n2 = make_node(b"r1", &n1.hash, MessageType::Request,  "support-agent", "infer.ollama",  101);
        let n3 = make_node(b"r2", &n2.hash, MessageType::Response, "infer.ollama",  "support-agent", 102);
        let n4 = make_node(b"d",  &n3.hash, MessageType::Delegate, "support-agent", "log-analyst",   103);
        let n5 = make_node(b"i2", &n4.hash, MessageType::Invoke,   "support-agent", "log-analyst",   104);
        let n6 = make_node(b"r3", &n5.hash, MessageType::Request,  "log-analyst",   "infer.ollama",  105);
        let n7 = make_node(b"r4", &n6.hash, MessageType::Response, "infer.ollama",  "log-analyst",   106);
        let n8 = make_node(b"c2", &n7.hash, MessageType::Complete, "log-analyst",   "support-agent", 107);
        let n9 = make_node(b"c1", &n8.hash, MessageType::Complete, "support-agent", "cli",           108);

        let route = Route::from_dag_nodes("sess-1".to_string(),
            vec![n1, n2, n3, n4, n5, n6, n7, n8, n9]);

        assert_eq!(route.stop_count(), 9);
        assert_eq!(route.duration_secs(), Some(8.0));

        let agents = route.agents();
        assert_eq!(agents, vec!["cli", "support-agent", "infer.ollama", "log-analyst"]);

        // Every message type is visible
        let types: Vec<MessageType> = route.stops().iter().map(|s| s.message_type).collect();
        assert_eq!(types, vec![
            MessageType::Invoke,
            MessageType::Request,
            MessageType::Response,
            MessageType::Delegate,
            MessageType::Invoke,
            MessageType::Request,
            MessageType::Response,
            MessageType::Complete,
            MessageType::Complete,
        ]);
    }
}
