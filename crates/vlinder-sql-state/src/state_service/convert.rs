//! Conversions between domain DagNode and protobuf DagNode.

use std::str::FromStr;

use chrono::{DateTime, Utc};

use super::proto;
use vlinder_core::domain::{DagNode, MessageType, SessionSummary, Timeline};

// =============================================================================
// DagNode → proto::DagNode
// =============================================================================

impl From<DagNode> for proto::DagNode {
    fn from(node: DagNode) -> Self {
        Self {
            hash: node.hash,
            parent_hash: node.parent_hash,
            message_type: node.message_type.as_str().to_string(),
            sender: node.from,
            receiver: node.to,
            session_id: node.session_id,
            submission_id: node.submission_id,
            payload: node.payload,
            diagnostics: node.diagnostics,
            stderr: node.stderr,
            created_at: node.created_at.to_rfc3339(),
            state: node.state,
            protocol_version: node.protocol_version,
            checkpoint: node.checkpoint,
            operation: node.operation,
        }
    }
}

impl From<&DagNode> for proto::DagNode {
    fn from(node: &DagNode) -> Self {
        Self {
            hash: node.hash.clone(),
            parent_hash: node.parent_hash.clone(),
            message_type: node.message_type.as_str().to_string(),
            sender: node.from.clone(),
            receiver: node.to.clone(),
            session_id: node.session_id.clone(),
            submission_id: node.submission_id.clone(),
            payload: node.payload.clone(),
            diagnostics: node.diagnostics.clone(),
            stderr: node.stderr.clone(),
            created_at: node.created_at.to_rfc3339(),
            state: node.state.clone(),
            protocol_version: node.protocol_version.clone(),
            checkpoint: node.checkpoint.clone(),
            operation: node.operation.clone(),
        }
    }
}

// =============================================================================
// proto::DagNode → DagNode
// =============================================================================

impl TryFrom<proto::DagNode> for DagNode {
    type Error = String;

    fn try_from(node: proto::DagNode) -> Result<Self, Self::Error> {
        let message_type = MessageType::from_str(&node.message_type)
            .map_err(|_| format!("unknown message type: {}", node.message_type))?;

        let created_at: DateTime<Utc> = node
            .created_at
            .parse()
            .map_err(|e| format!("invalid created_at: {}", e))?;

        Ok(Self {
            hash: node.hash,
            parent_hash: node.parent_hash,
            message_type,
            from: node.sender,
            to: node.receiver,
            session_id: node.session_id,
            submission_id: node.submission_id,
            payload: node.payload,
            diagnostics: node.diagnostics,
            stderr: node.stderr,
            created_at,
            state: node.state,
            protocol_version: node.protocol_version,
            checkpoint: node.checkpoint,
            operation: node.operation,
        })
    }
}

// =============================================================================
// Timeline → proto::Timeline
// =============================================================================

impl From<Timeline> for proto::Timeline {
    fn from(tl: Timeline) -> Self {
        Self {
            id: tl.id,
            branch_name: tl.branch_name,
            session_id: tl.session_id,
            parent_timeline_id: tl.parent_timeline_id,
            fork_point: tl.fork_point,
            created_at: tl.created_at.to_rfc3339(),
            broken_at: tl.broken_at.map(|dt| dt.to_rfc3339()),
        }
    }
}

// =============================================================================
// proto::Timeline → Timeline
// =============================================================================

impl TryFrom<proto::Timeline> for Timeline {
    type Error = String;

    fn try_from(tl: proto::Timeline) -> Result<Self, Self::Error> {
        let created_at: DateTime<Utc> = tl
            .created_at
            .parse()
            .map_err(|e| format!("invalid created_at: {}", e))?;
        let broken_at = tl
            .broken_at
            .map(|s| s.parse::<DateTime<Utc>>())
            .transpose()
            .map_err(|e| format!("invalid broken_at: {}", e))?;

        Ok(Self {
            id: tl.id,
            branch_name: tl.branch_name,
            session_id: tl.session_id,
            parent_timeline_id: tl.parent_timeline_id,
            fork_point: tl.fork_point,
            created_at,
            broken_at,
        })
    }
}

// =============================================================================
// SessionSummary → proto::SessionSummary
// =============================================================================

impl From<SessionSummary> for proto::SessionSummary {
    fn from(s: SessionSummary) -> Self {
        Self {
            session_id: s.session_id,
            agent_name: s.agent_name,
            started_at: s.started_at.to_rfc3339(),
            message_count: s.message_count as u64,
            is_open: s.is_open,
        }
    }
}

// =============================================================================
// proto::SessionSummary → SessionSummary
// =============================================================================

impl TryFrom<proto::SessionSummary> for SessionSummary {
    type Error = String;

    fn try_from(s: proto::SessionSummary) -> Result<Self, Self::Error> {
        let started_at: DateTime<Utc> = s
            .started_at
            .parse()
            .map_err(|e| format!("invalid started_at: {}", e))?;

        Ok(Self {
            session_id: s.session_id,
            agent_name: s.agent_name,
            started_at,
            message_count: s.message_count as usize,
            is_open: s.is_open,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_dag_node() -> DagNode {
        DagNode {
            hash: "abc123".to_string(),
            parent_hash: "parent456".to_string(),
            message_type: MessageType::Invoke,
            from: "cli".to_string(),
            to: "agent-echo".to_string(),
            session_id: "sess-001".to_string(),
            submission_id: "sub-001".to_string(),
            payload: b"hello".to_vec(),
            diagnostics: b"{\"version\":\"0.1.0\"}".to_vec(),
            stderr: b"some stderr".to_vec(),
            created_at: Utc::now(),
            state: Some("state-hash-abc".to_string()),
            protocol_version: "0.1.0".to_string(),
            checkpoint: None,
            operation: None,
        }
    }

    #[test]
    fn dag_node_round_trip() {
        let original = sample_dag_node();
        let proto_node: proto::DagNode = original.clone().into();
        let recovered: DagNode = proto_node.try_into().unwrap();

        assert_eq!(recovered.hash, original.hash);
        assert_eq!(recovered.parent_hash, original.parent_hash);
        assert_eq!(recovered.message_type, original.message_type);
        assert_eq!(recovered.from, original.from);
        assert_eq!(recovered.to, original.to);
        assert_eq!(recovered.session_id, original.session_id);
        assert_eq!(recovered.submission_id, original.submission_id);
        assert_eq!(recovered.payload, original.payload);
        assert_eq!(recovered.diagnostics, original.diagnostics);
        assert_eq!(recovered.stderr, original.stderr);
        assert_eq!(recovered.state, original.state);
        assert_eq!(recovered.protocol_version, original.protocol_version);
    }

    #[test]
    fn dag_node_without_state_round_trips() {
        let mut node = sample_dag_node();
        node.state = None;

        let proto_node: proto::DagNode = node.clone().into();
        let recovered: DagNode = proto_node.try_into().unwrap();

        assert_eq!(recovered.state, None);
    }

    #[test]
    fn message_type_round_trip() {
        for mt in [
            MessageType::Invoke,
            MessageType::Request,
            MessageType::Response,
            MessageType::Complete,
            MessageType::Delegate,
            MessageType::Repair,
        ] {
            let node = DagNode {
                message_type: mt,
                ..sample_dag_node()
            };
            let proto_node: proto::DagNode = node.into();
            let recovered: DagNode = proto_node.try_into().unwrap();
            assert_eq!(recovered.message_type, mt);
        }
    }

    #[test]
    fn invalid_message_type_fails() {
        let proto_node = proto::DagNode {
            message_type: "bogus".to_string(),
            created_at: Utc::now().to_rfc3339(),
            ..Default::default()
        };
        assert!(DagNode::try_from(proto_node).is_err());
    }

    #[test]
    fn invalid_created_at_fails() {
        let proto_node = proto::DagNode {
            message_type: "invoke".to_string(),
            created_at: "not-a-date".to_string(),
            ..Default::default()
        };
        assert!(DagNode::try_from(proto_node).is_err());
    }
}
