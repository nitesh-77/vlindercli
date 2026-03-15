//! Conversions between domain DagNode and protobuf DagNode.

use chrono::{DateTime, Utc};

use super::proto;
use vlinder_core::domain::{
    DagNode, DagNodeId, ObservableMessage, SessionId, SessionSummary, Timeline,
};

// =============================================================================
// DagNode → proto::DagNode
// =============================================================================

fn dag_node_to_proto(node: &DagNode) -> proto::DagNode {
    let (from, to) = node.message.from_to();
    let message_blob = serde_json::to_string(&node.message).ok();
    proto::DagNode {
        hash: node.id.to_string(),
        parent_hash: node.parent_id.to_string(),
        message_type: node.message_type().as_str().to_string(),
        sender: from,
        receiver: to,
        session_id: node.session_id().as_str().to_string(),
        submission_id: node.submission_id().to_string(),
        payload: node.payload().to_vec(),
        diagnostics: node.message.diagnostics_json(),
        stderr: node.message.stderr().to_vec(),
        created_at: node.created_at.to_rfc3339(),
        state: node.message.state().map(|s| s.to_string()),
        protocol_version: node.protocol_version().to_string(),
        checkpoint: node.message.checkpoint().map(|s| s.to_string()),
        operation: node.message.operation().map(|s| s.to_string()),
        message_blob,
    }
}

impl From<DagNode> for proto::DagNode {
    fn from(node: DagNode) -> Self {
        dag_node_to_proto(&node)
    }
}

impl From<&DagNode> for proto::DagNode {
    fn from(node: &DagNode) -> Self {
        dag_node_to_proto(node)
    }
}

// =============================================================================
// proto::DagNode → DagNode
// =============================================================================

impl TryFrom<proto::DagNode> for DagNode {
    type Error = String;

    fn try_from(node: proto::DagNode) -> Result<Self, Self::Error> {
        let created_at: DateTime<Utc> = node
            .created_at
            .parse()
            .map_err(|e| format!("invalid created_at: {}", e))?;

        let message: ObservableMessage = node
            .message_blob
            .as_ref()
            .ok_or_else(|| "missing message_blob".to_string())
            .and_then(|blob| {
                serde_json::from_str(blob).map_err(|e| format!("invalid message_blob JSON: {}", e))
            })?;

        Ok(Self {
            id: DagNodeId::from(node.hash),
            parent_id: DagNodeId::from(node.parent_hash),
            created_at,
            message,
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
            session_id: tl.session_id.as_str().to_string(),
            parent_timeline_id: tl.parent_timeline_id,
            fork_point: tl.fork_point.map(|fp| fp.to_string()),
            created_at: tl.created_at.to_rfc3339(),
            broken_at: tl.broken_at.map(|dt| dt.to_rfc3339()),
            head: None,
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
            session_id: SessionId::try_from(tl.session_id)?,
            parent_timeline_id: tl.parent_timeline_id,
            fork_point: tl.fork_point.map(DagNodeId::from),
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
            session_id: s.session_id.as_str().to_string(),
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
            session_id: SessionId::try_from(s.session_id)?,
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
    use vlinder_core::domain::workers::dag::build_dag_node;
    use vlinder_core::domain::{
        AgentId, DagNodeId, HarnessType, InvokeDiagnostics, InvokeMessage, RuntimeType, SessionId,
        SubmissionId, TimelineId,
    };

    fn sample_dag_node() -> DagNode {
        let msg: ObservableMessage = InvokeMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-001".to_string()),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("agent-echo"),
            b"hello".to_vec(),
            Some("state-hash-abc".to_string()),
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        )
        .into();
        build_dag_node(&msg, &DagNodeId::from("parent456".to_string()))
    }

    #[test]
    fn dag_node_round_trip() {
        let original = sample_dag_node();
        let proto_node: proto::DagNode = original.clone().into();
        let recovered: DagNode = proto_node.try_into().unwrap();

        assert_eq!(recovered.id, original.id);
        assert_eq!(recovered.parent_id, original.parent_id);
        assert_eq!(recovered.message_type(), original.message_type());
        assert_eq!(recovered.session_id(), original.session_id());
        assert_eq!(recovered.submission_id(), original.submission_id());
        assert_eq!(recovered.payload(), original.payload());
        assert_eq!(recovered.message.state(), original.message.state());
        assert_eq!(recovered.protocol_version(), original.protocol_version());
    }

    #[test]
    fn dag_node_without_state_round_trips() {
        let msg: ObservableMessage = InvokeMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-001".to_string()),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("agent-echo"),
            b"hello".to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        )
        .into();
        let node = build_dag_node(&msg, &DagNodeId::root());

        let proto_node: proto::DagNode = node.clone().into();
        let recovered: DagNode = proto_node.try_into().unwrap();

        assert_eq!(recovered.message.state(), None);
    }

    #[test]
    fn missing_message_blob_fails() {
        let proto_node = proto::DagNode {
            message_type: "invoke".to_string(),
            created_at: Utc::now().to_rfc3339(),
            message_blob: None,
            ..Default::default()
        };
        assert!(DagNode::try_from(proto_node).is_err());
    }

    #[test]
    fn invalid_created_at_fails() {
        let proto_node = proto::DagNode {
            message_type: "invoke".to_string(),
            created_at: "not-a-date".to_string(),
            message_blob: Some("{}".to_string()),
            ..Default::default()
        };
        assert!(DagNode::try_from(proto_node).is_err());
    }
}
