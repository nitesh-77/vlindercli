//! gRPC client implementing the DagStore trait.

use std::sync::Mutex;
use tonic::transport::Channel;

use crate::domain::{DagNode, DagStore};
use super::proto::{self, state_service_client::StateServiceClient};

/// DagStore implementation that makes gRPC calls to a remote State Service.
pub struct GrpcStateClient {
    client: Mutex<StateServiceClient<Channel>>,
    runtime: tokio::runtime::Runtime,
}

impl GrpcStateClient {
    /// Connect to a state service server.
    pub fn connect(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Runtime::new()?;
        let client = runtime.block_on(async {
            StateServiceClient::connect(addr.to_string()).await
        })?;

        Ok(Self {
            client: Mutex::new(client),
            runtime,
        })
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

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .insert_node(request)
                .await
        }).map_err(|e| e.to_string())?;

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

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .get_node(request)
                .await
        }).map_err(|e| e.to_string())?;

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

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .get_session_nodes(request)
                .await
        }).map_err(|e| e.to_string())?;

        response.into_inner().nodes
            .into_iter()
            .map(|n| n.try_into())
            .collect()
    }

    fn get_children(&self, parent_hash: &str) -> Result<Vec<DagNode>, String> {
        let request = proto::GetChildrenRequest {
            parent_hash: parent_hash.to_string(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .get_children(request)
                .await
        }).map_err(|e| e.to_string())?;

        response.into_inner().nodes
            .into_iter()
            .map(|n| n.try_into())
            .collect()
    }

    fn latest_state(&self, agent_name: &str) -> Result<Option<String>, String> {
        let request = proto::LatestStateRequest {
            agent_name: agent_name.to_string(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .latest_state(request)
                .await
        }).map_err(|e| e.to_string())?;

        Ok(response.into_inner().state)
    }
}
