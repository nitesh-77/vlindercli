//! gRPC server wrapping the DagStore trait.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use super::proto::{
    self, state_service_server::StateService, CreateBranchRequest, CreateBranchResponse,
    CreateSessionRequest, CreateSessionResponse, GetBranchByIdRequest, GetBranchByNameRequest,
    GetBranchResponse, GetBranchesForSessionRequest, GetBranchesForSessionResponse,
    GetChildrenRequest, GetChildrenResponse, GetNodeByPrefixRequest, GetNodeRequest,
    GetNodeResponse, GetNodesBySubmissionRequest, GetNodesBySubmissionResponse,
    GetSessionByNameRequest, GetSessionNodesRequest, GetSessionNodesResponse, GetSessionRequest,
    GetSessionResponse, InsertNodeRequest, InsertNodeResponse, LatestNodeHashRequest,
    LatestNodeHashResponse, LatestNodeOnBranchRequest, LatestNodeOnBranchResponse,
    LatestStateRequest, LatestStateResponse, ListSessionsRequest, ListSessionsResponse,
    PingRequest, SemVer, SetCheckoutStateRequest, SetCheckoutStateResponse,
};
use vlinder_core::domain::{DagNodeId, DagStore, MessageType, SessionId};

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
    async fn ping(&self, _request: Request<PingRequest>) -> Result<Response<SemVer>, Status> {
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
        let proto_node = req
            .node
            .ok_or_else(|| Status::invalid_argument("missing node"))?;

        let node = proto_node
            .try_into()
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

        let node = self
            .store
            .get_node(&DagNodeId::from(req.hash))
            .map_err(Status::internal)?
            .map(|n| n.into());

        Ok(Response::new(GetNodeResponse { node }))
    }

    async fn get_session_nodes(
        &self,
        request: Request<GetSessionNodesRequest>,
    ) -> Result<Response<GetSessionNodesResponse>, Status> {
        let req = request.into_inner();

        let nodes = self
            .store
            .get_session_nodes(
                &SessionId::try_from(req.session_id).map_err(Status::invalid_argument)?,
            )
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

        let nodes = self
            .store
            .get_children(&DagNodeId::from(req.parent_hash))
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

        let state = self
            .store
            .latest_state(&req.agent_name)
            .map_err(Status::internal)?;

        Ok(Response::new(LatestStateResponse { state }))
    }

    async fn latest_node_hash(
        &self,
        request: Request<LatestNodeHashRequest>,
    ) -> Result<Response<LatestNodeHashResponse>, Status> {
        let req = request.into_inner();

        let hash = self
            .store
            .latest_node_hash(
                &SessionId::try_from(req.session_id).map_err(Status::invalid_argument)?,
            )
            .map_err(Status::internal)?
            .map(|id| id.to_string());

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

    // -------------------------------------------------------------------------
    // Branch RPCs
    // -------------------------------------------------------------------------

    async fn create_branch(
        &self,
        request: Request<CreateBranchRequest>,
    ) -> Result<Response<CreateBranchResponse>, Status> {
        let req = request.into_inner();
        let fork_point = req.fork_point.map(DagNodeId::from);
        let id = self
            .store
            .create_branch(
                &req.name,
                &SessionId::try_from(req.session_id).map_err(Status::invalid_argument)?,
                fork_point.as_ref(),
            )
            .map_err(Status::internal)?;
        Ok(Response::new(CreateBranchResponse { id }))
    }

    async fn get_branch_by_name(
        &self,
        request: Request<GetBranchByNameRequest>,
    ) -> Result<Response<GetBranchResponse>, Status> {
        let req = request.into_inner();
        let branch = self
            .store
            .get_branch_by_name(&req.name)
            .map_err(Status::internal)?
            .map(|b| b.into());
        Ok(Response::new(GetBranchResponse { branch }))
    }

    async fn get_branch(
        &self,
        request: Request<GetBranchByIdRequest>,
    ) -> Result<Response<GetBranchResponse>, Status> {
        let req = request.into_inner();
        let branch = self
            .store
            .get_branch(req.id)
            .map_err(Status::internal)?
            .map(|b| b.into());
        Ok(Response::new(GetBranchResponse { branch }))
    }

    // -------------------------------------------------------------------------
    // Session query RPCs
    // -------------------------------------------------------------------------

    async fn list_sessions(
        &self,
        _request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let sessions = self
            .store
            .list_sessions()
            .map_err(Status::internal)?
            .into_iter()
            .map(|s| s.into())
            .collect();
        Ok(Response::new(ListSessionsResponse { sessions }))
    }

    async fn get_nodes_by_submission(
        &self,
        request: Request<GetNodesBySubmissionRequest>,
    ) -> Result<Response<GetNodesBySubmissionResponse>, Status> {
        let req = request.into_inner();
        let nodes = self
            .store
            .get_nodes_by_submission(&req.submission_id)
            .map_err(Status::internal)?
            .into_iter()
            .map(|n| n.into())
            .collect();
        Ok(Response::new(GetNodesBySubmissionResponse { nodes }))
    }

    async fn get_node_by_prefix(
        &self,
        request: Request<GetNodeByPrefixRequest>,
    ) -> Result<Response<GetNodeResponse>, Status> {
        let req = request.into_inner();
        let node = self
            .store
            .get_node_by_prefix(&req.prefix)
            .map_err(Status::internal)?
            .map(|n| n.into());
        Ok(Response::new(GetNodeResponse { node }))
    }

    async fn get_branches_for_session(
        &self,
        request: Request<GetBranchesForSessionRequest>,
    ) -> Result<Response<GetBranchesForSessionResponse>, Status> {
        let req = request.into_inner();
        let branches = self
            .store
            .get_branches_for_session(
                &SessionId::try_from(req.session_id).map_err(Status::invalid_argument)?,
            )
            .map_err(Status::internal)?
            .into_iter()
            .map(|b| b.into())
            .collect();
        Ok(Response::new(GetBranchesForSessionResponse { branches }))
    }

    // -------------------------------------------------------------------------
    // Session CRUD RPCs
    // -------------------------------------------------------------------------

    async fn create_session(
        &self,
        request: Request<CreateSessionRequest>,
    ) -> Result<Response<CreateSessionResponse>, Status> {
        let req = request.into_inner();
        let session_proto = req
            .session
            .ok_or_else(|| Status::invalid_argument("missing session"))?;
        let session_id = SessionId::try_from(session_proto.id).map_err(Status::invalid_argument)?;
        let session = vlinder_core::domain::Session::new(
            session_id,
            &session_proto.agent_name,
            session_proto.default_branch,
        );
        match self.store.create_session(&vlinder_core::domain::Session {
            name: session_proto.name,
            ..session
        }) {
            Ok(()) => Ok(Response::new(CreateSessionResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(CreateSessionResponse {
                success: false,
                error: Some(e),
            })),
        }
    }

    async fn get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = SessionId::try_from(req.session_id).map_err(Status::invalid_argument)?;
        let session = self
            .store
            .get_session(&session_id)
            .map_err(Status::internal)?;
        Ok(Response::new(GetSessionResponse {
            session: session.map(session_to_proto),
        }))
    }

    async fn get_session_by_name(
        &self,
        request: Request<GetSessionByNameRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .store
            .get_session_by_name(&req.name)
            .map_err(Status::internal)?;
        Ok(Response::new(GetSessionResponse {
            session: session.map(session_to_proto),
        }))
    }

    async fn latest_node_on_branch(
        &self,
        request: Request<LatestNodeOnBranchRequest>,
    ) -> Result<Response<LatestNodeOnBranchResponse>, Status> {
        let req = request.into_inner();
        let message_type = req
            .message_type
            .map(|s| s.parse::<MessageType>())
            .transpose()
            .map_err(Status::invalid_argument)?;
        let node = self
            .store
            .latest_node_on_branch(req.branch_id, message_type)
            .map_err(Status::internal)?
            .map(|n| n.into());
        Ok(Response::new(LatestNodeOnBranchResponse { node }))
    }
}

fn session_to_proto(s: vlinder_core::domain::Session) -> proto::SessionProto {
    proto::SessionProto {
        id: s.id.as_str().to_string(),
        name: s.name,
        agent_name: s.agent,
        default_branch: s.default_branch,
        created_at: s.created_at.to_rfc3339(),
    }
}
