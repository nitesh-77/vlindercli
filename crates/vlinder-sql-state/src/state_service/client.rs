//! gRPC client implementing the DagStore trait.

use tonic::transport::Channel;

use super::proto::{self, state_service_client::StateServiceClient};
use vlinder_core::domain::{DagNode, DagStore, Timeline};

/// DagStore implementation that makes gRPC calls to a remote State Service.
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

    fn get_node(&self, hash: &str) -> Result<Option<DagNode>, String> {
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

    fn get_session_nodes(&self, session_id: &str) -> Result<Vec<DagNode>, String> {
        let request = proto::GetSessionNodesRequest {
            session_id: session_id.to_string(),
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
            .map(|n| n.try_into())
            .collect()
    }

    fn get_children(&self, parent_hash: &str) -> Result<Vec<DagNode>, String> {
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
            .map(|n| n.try_into())
            .collect()
    }

    fn latest_state(&self, agent_name: &str) -> Result<Option<String>, String> {
        let request = proto::LatestStateRequest {
            agent_name: agent_name.to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.latest_state(request).await })
            .map_err(|e| e.to_string())?;

        Ok(response.into_inner().state)
    }

    fn latest_node_hash(&self, session_id: &str) -> Result<Option<String>, String> {
        let request = proto::LatestNodeHashRequest {
            session_id: session_id.to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.latest_node_hash(request).await })
            .map_err(|e| e.to_string())?;

        Ok(response.into_inner().hash)
    }

    fn set_checkout_state(&self, agent_name: &str, state: &str) -> Result<(), String> {
        let request = proto::SetCheckoutStateRequest {
            agent_name: agent_name.to_string(),
            state: state.to_string(),
        };

        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.set_checkout_state(request).await })
            .map_err(|e| e.to_string())?;

        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
        }
    }

    // -------------------------------------------------------------------------
    // Timeline methods (ADR 093)
    // -------------------------------------------------------------------------

    fn ensure_main_timeline(&self) -> Result<i64, String> {
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async {
                client
                    .ensure_main_timeline(proto::EnsureMainTimelineRequest {})
                    .await
            })
            .map_err(|e| e.to_string())?;
        Ok(response.into_inner().id)
    }

    fn create_timeline(
        &self,
        branch_name: &str,
        parent_id: Option<i64>,
        fork_point: Option<&str>,
    ) -> Result<i64, String> {
        let request = proto::CreateTimelineRequest {
            branch_name: branch_name.to_string(),
            parent_id,
            fork_point: fork_point.map(|s| s.to_string()),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.create_timeline(request).await })
            .map_err(|e| e.to_string())?;
        Ok(response.into_inner().id)
    }

    fn get_timeline_by_branch(&self, branch_name: &str) -> Result<Option<Timeline>, String> {
        let request = proto::GetTimelineByBranchRequest {
            branch_name: branch_name.to_string(),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_timeline_by_branch(request).await })
            .map_err(|e| e.to_string())?;

        match response.into_inner().timeline {
            Some(tl) => Ok(Some(tl.try_into()?)),
            None => Ok(None),
        }
    }

    fn get_timeline(&self, id: i64) -> Result<Option<Timeline>, String> {
        let request = proto::GetTimelineByIdRequest { id };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.get_timeline(request).await })
            .map_err(|e| e.to_string())?;

        match response.into_inner().timeline {
            Some(tl) => Ok(Some(tl.try_into()?)),
            None => Ok(None),
        }
    }

    fn seal_timeline(&self, id: i64) -> Result<(), String> {
        let request = proto::SealTimelineRequest { id };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.seal_timeline(request).await })
            .map_err(|e| e.to_string())?;

        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
        }
    }

    fn rename_timeline(&self, id: i64, new_name: &str) -> Result<(), String> {
        let request = proto::RenameTimelineRequest {
            id,
            new_name: new_name.to_string(),
        };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.rename_timeline(request).await })
            .map_err(|e| e.to_string())?;

        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
        }
    }

    fn is_timeline_sealed(&self, id: i64) -> Result<bool, String> {
        let request = proto::IsTimelineSealedRequest { id };
        let mut client = self.client.clone();
        let response = self
            .runtime
            .block_on(async { client.is_timeline_sealed(request).await })
            .map_err(|e| e.to_string())?;
        Ok(response.into_inner().sealed)
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
            .map(|s| s.try_into())
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
            .map(|n| n.try_into())
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
}
