//! gRPC server wrapping the DagStore trait.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::domain::DagStore;
use super::proto::{
    self,
    state_service_server::StateService,
    PingRequest, SemVer,
    InsertNodeRequest, InsertNodeResponse,
    GetNodeRequest, GetNodeResponse,
    GetSessionNodesRequest, GetSessionNodesResponse,
    GetChildrenRequest, GetChildrenResponse,
    LatestStateRequest, LatestStateResponse,
    LatestNodeHashRequest, LatestNodeHashResponse,
    SetCheckoutStateRequest, SetCheckoutStateResponse,
};

/// gRPC server that wraps a DagStore implementation.
pub struct StateServiceServer {
    store: Arc<dyn DagStore>,
}

impl StateServiceServer {
    pub fn new(store: Arc<dyn DagStore>) -> Self {
        Self { store }
    }

    /// Create a tonic service from this server.
    pub fn into_service(self) -> proto::state_service_server::StateServiceServer<Self> {
        proto::state_service_server::StateServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl StateService for StateServiceServer {
    async fn ping(
        &self,
        _request: Request<PingRequest>,
    ) -> Result<Response<SemVer>, Status> {
        Ok(Response::new(SemVer {
            major: 0,
            minor: 0,
            patch: 1,
        }))
    }

    async fn insert_node(
        &self,
        request: Request<InsertNodeRequest>,
    ) -> Result<Response<InsertNodeResponse>, Status> {
        let req = request.into_inner();
        let proto_node = req.node
            .ok_or_else(|| Status::invalid_argument("missing node"))?;

        let node = proto_node.try_into()
            .map_err(|e: String| Status::invalid_argument(e))?;

        match self.store.insert_node(&node) {
            Ok(()) => Ok(Response::new(InsertNodeResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(InsertNodeResponse {
                success: false,
                error: Some(e),
            })),
        }
    }

    async fn get_node(
        &self,
        request: Request<GetNodeRequest>,
    ) -> Result<Response<GetNodeResponse>, Status> {
        let req = request.into_inner();

        let node = self.store.get_node(&req.hash)
            .map_err(Status::internal)?
            .map(|n| n.into());

        Ok(Response::new(GetNodeResponse { node }))
    }

    async fn get_session_nodes(
        &self,
        request: Request<GetSessionNodesRequest>,
    ) -> Result<Response<GetSessionNodesResponse>, Status> {
        let req = request.into_inner();

        let nodes = self.store.get_session_nodes(&req.session_id)
            .map_err(Status::internal)?
            .into_iter()
            .map(|n| n.into())
            .collect();

        Ok(Response::new(GetSessionNodesResponse { nodes }))
    }

    async fn get_children(
        &self,
        request: Request<GetChildrenRequest>,
    ) -> Result<Response<GetChildrenResponse>, Status> {
        let req = request.into_inner();

        let nodes = self.store.get_children(&req.parent_hash)
            .map_err(Status::internal)?
            .into_iter()
            .map(|n| n.into())
            .collect();

        Ok(Response::new(GetChildrenResponse { nodes }))
    }

    async fn latest_state(
        &self,
        request: Request<LatestStateRequest>,
    ) -> Result<Response<LatestStateResponse>, Status> {
        let req = request.into_inner();

        let state = self.store.latest_state(&req.agent_name)
            .map_err(Status::internal)?;

        Ok(Response::new(LatestStateResponse { state }))
    }

    async fn latest_node_hash(
        &self,
        request: Request<LatestNodeHashRequest>,
    ) -> Result<Response<LatestNodeHashResponse>, Status> {
        let req = request.into_inner();

        let hash = self.store.latest_node_hash(&req.session_id)
            .map_err(Status::internal)?;

        Ok(Response::new(LatestNodeHashResponse { hash }))
    }

    async fn set_checkout_state(
        &self,
        request: Request<SetCheckoutStateRequest>,
    ) -> Result<Response<SetCheckoutStateResponse>, Status> {
        let req = request.into_inner();

        match self.store.set_checkout_state(&req.agent_name, &req.state) {
            Ok(()) => Ok(Response::new(SetCheckoutStateResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(SetCheckoutStateResponse {
                success: false,
                error: Some(e),
            })),
        }
    }
}
