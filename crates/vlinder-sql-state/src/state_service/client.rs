//! gRPC client implementing the `DagStore` trait.

use tonic::transport::Channel;

use super::proto::{self, state_service_client::StateServiceClient};
use vlinder_core::domain::{Branch, BranchId, DagNode, DagNodeId, DagStore};

/// `DagStore` implementation that makes gRPC calls to a remote State Service.
pub struct GrpcStateClient {
    client: StateServiceClient<Channel>,
    runtime: tokio::runtime::Runtime,
}

impl GrpcStateClient {
    /// Connect to a state service server.
    pub fn connect(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Runtime::new()?;
        let client =
            runtime.block_on(async { StateServiceClient::connect(addr.to_string()).await })?;

        Ok(Self { client, runtime })
    }
}

/// Ping a state service at the given address, returning its protocol version.
///
/// Creates a temporary connection and sends a Ping. Returns the server's
/// version on success, None on any connection or transport error.
pub fn ping_state_service(addr: &str) -> Option<(u32, u32, u32)> {
    let Ok(runtime) = tokio::runtime::Runtime::new() else {
        return None;
    };

    runtime.block_on(async {
        let Ok(mut client) = StateServiceClient::connect(addr.to_string()).await else {
            return None;
        };
        client.ping(proto::PingRequest {}).await.ok().map(|r| {
            let v = r.into_inner();
            (v.major, v.minor, v.patch)
        })
    })
}

impl DagStore for GrpcStateClient {
    fn insert_node(&self, node: &DagNode) -> Result<(), String> {
        let proto_node: proto::DagNode = node.into();
        let request = proto::InsertNodeRequest {
            node: Some(proto_node),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.insert_node(request).await })
            .map_err(|e| e.to_string())?;

        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
        }
    }

    fn get_node(&self, hash: &DagNodeId) -> Result<Option<DagNode>, String> {
        let request = proto::GetNodeRequest {
            hash: hash.to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_node(request).await })
            .map_err(|e| e.to_string())?;

        match response.into_inner().node {
            Some(proto_node) => {
                let node = proto_node.try_into()?;
                Ok(Some(node))
            }
            None => Ok(None),
        }
    }

    fn get_session_nodes(
        &self,
        session_id: &vlinder_core::domain::SessionId,
    ) -> Result<Vec<DagNode>, String> {
        let request = proto::GetSessionNodesRequest {
            session_id: session_id.as_str().to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_session_nodes(request).await })
            .map_err(|e| e.to_string())?;

        response
            .into_inner()
            .nodes
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect()
    }

    fn get_children(&self, parent_hash: &DagNodeId) -> Result<Vec<DagNode>, String> {
        let request = proto::GetChildrenRequest {
            parent_hash: parent_hash.to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_children(request).await })
            .map_err(|e| e.to_string())?;

        response
            .into_inner()
            .nodes
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect()
    }

    // -------------------------------------------------------------------------
    // Branch methods
    // -------------------------------------------------------------------------

    fn create_branch(
        &self,
        name: &str,
        session_id: &vlinder_core::domain::SessionId,
        fork_point: Option<&DagNodeId>,
    ) -> Result<BranchId, String> {
        let request = proto::CreateBranchRequest {
            name: name.to_string(),
            session_id: session_id.as_str().to_string(),
            fork_point: fork_point.map(std::string::ToString::to_string),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.create_branch(request).await })
            .map_err(|e| e.to_string())?;
        Ok(BranchId::from(response.into_inner().id))
    }

    fn get_branch_by_name(&self, name: &str) -> Result<Option<Branch>, String> {
        let request = proto::GetBranchByNameRequest {
            name: name.to_string(),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_branch_by_name(request).await })
            .map_err(|e| e.to_string())?;

        match response.into_inner().branch {
            Some(b) => Ok(Some(b.try_into()?)),
            None => Ok(None),
        }
    }

    fn get_branch(&self, id: BranchId) -> Result<Option<Branch>, String> {
        let request = proto::GetBranchByIdRequest { id: id.as_i64() };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_branch(request).await })
            .map_err(|e| e.to_string())?;

        match response.into_inner().branch {
            Some(b) => Ok(Some(b.try_into()?)),
            None => Ok(None),
        }
    }

    fn list_sessions(&self) -> Result<Vec<vlinder_core::domain::SessionSummary>, String> {
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.list_sessions(proto::ListSessionsRequest {}).await })
            .map_err(|e| e.to_string())?;

        response
            .into_inner()
            .sessions
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect()
    }

    fn get_nodes_by_submission(&self, submission_id: &str) -> Result<Vec<DagNode>, String> {
        let request = proto::GetNodesBySubmissionRequest {
            submission_id: submission_id.to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_nodes_by_submission(request).await })
            .map_err(|e| e.to_string())?;

        response
            .into_inner()
            .nodes
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect()
    }

    fn get_node_by_prefix(&self, prefix: &str) -> Result<Option<DagNode>, String> {
        let request = proto::GetNodeByPrefixRequest {
            prefix: prefix.to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_node_by_prefix(request).await })
            .map_err(|e| e.to_string())?;

        match response.into_inner().node {
            Some(n) => Ok(Some(n.try_into()?)),
            None => Ok(None),
        }
    }

    fn get_branches_for_session(
        &self,
        session_id: &vlinder_core::domain::SessionId,
    ) -> Result<Vec<Branch>, String> {
        let request = proto::GetBranchesForSessionRequest {
            session_id: session_id.as_str().to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_branches_for_session(request).await })
            .map_err(|e| e.to_string())?;

        response
            .into_inner()
            .branches
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect()
    }

    fn latest_node_on_branch(
        &self,
        branch_id: BranchId,
        message_type: Option<vlinder_core::domain::MessageType>,
    ) -> Result<Option<vlinder_core::domain::DagNode>, String> {
        let request = proto::LatestNodeOnBranchRequest {
            branch_id: branch_id.as_i64(),
            message_type: message_type.map(|mt| mt.as_str().to_string()),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.latest_node_on_branch(request).await })
            .map_err(|e| e.to_string())?;

        match response.into_inner().node {
            Some(n) => Ok(Some(n.try_into()?)),
            None => Ok(None),
        }
    }

    fn rename_branch(
        &self,
        id: vlinder_core::domain::BranchId,
        new_name: &str,
    ) -> Result<(), String> {
        let request = proto::RenameBranchRequest {
            id: id.as_i64(),
            new_name: new_name.to_string(),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.rename_branch(request).await })
            .map_err(|e| e.to_string())?;
        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
        }
    }

    fn seal_branch(
        &self,
        id: vlinder_core::domain::BranchId,
        broken_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), String> {
        let request = proto::SealBranchRequest {
            id: id.as_i64(),
            broken_at: broken_at.to_rfc3339(),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.seal_branch(request).await })
            .map_err(|e| e.to_string())?;
        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
        }
    }

    fn update_session_default_branch(
        &self,
        session_id: &vlinder_core::domain::SessionId,
        branch_id: vlinder_core::domain::BranchId,
    ) -> Result<(), String> {
        let request = proto::UpdateSessionDefaultBranchRequest {
            session_id: session_id.as_str().to_string(),
            branch_id: branch_id.as_i64(),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.update_session_default_branch(request).await })
            .map_err(|e| e.to_string())?;
        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
        }
    }

    fn create_session(&self, session: &vlinder_core::domain::Session) -> Result<(), String> {
        let mut client = self.client.clone();
        let req = proto::CreateSessionRequest {
            session: Some(proto::SessionProto {
                id: session.id.as_str().to_string(),
                name: session.name.clone(),
                agent_name: session.agent.clone(),
                default_branch: session.default_branch.as_i64(),
                created_at: session.created_at.to_rfc3339(),
            }),
        };
        let resp = self
            .runtime
            .block_on(async { client.create_session(req).await })
            .map_err(|e| e.to_string())?
            .into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
        }
    }

    fn get_session(
        &self,
        session_id: &vlinder_core::domain::SessionId,
    ) -> Result<Option<vlinder_core::domain::Session>, String> {
        let mut client = self.client.clone();
        let req = proto::GetSessionRequest {
            session_id: session_id.as_str().to_string(),
        };
        let resp = self
            .runtime
            .block_on(async { client.get_session(req).await })
            .map_err(|e| e.to_string())?
            .into_inner();
        resp.session.map(proto_to_session).transpose()
    }

    fn get_session_by_name(
        &self,
        name: &str,
    ) -> Result<Option<vlinder_core::domain::Session>, String> {
        let mut client = self.client.clone();
        let req = proto::GetSessionByNameRequest {
            name: name.to_string(),
        };
        let resp = self
            .runtime
            .block_on(async { client.get_session_by_name(req).await })
            .map_err(|e| e.to_string())?
            .into_inner();
        resp.session.map(proto_to_session).transpose()
    }
}

fn proto_to_session(s: proto::SessionProto) -> Result<vlinder_core::domain::Session, String> {
    let id = vlinder_core::domain::SessionId::try_from(s.id)?;
    let created_at: chrono::DateTime<chrono::Utc> = s
        .created_at
        .parse()
        .map_err(|e| format!("invalid created_at: {e}"))?;
    Ok(vlinder_core::domain::Session {
        id,
        name: s.name,
        agent: s.agent_name,
        default_branch: BranchId::from(s.default_branch),
        created_at,
    })
}
