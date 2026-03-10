//! gRPC server wrapping the DagStore trait.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use super::proto::{
    self, state_service_server::StateService, CreateTimelineRequest, CreateTimelineResponse,
    GetChildrenRequest, GetChildrenResponse, GetNodeByPrefixRequest, GetNodeRequest,
    GetNodeResponse, GetNodesBySubmissionRequest, GetNodesBySubmissionResponse,
    GetSessionNodesRequest, GetSessionNodesResponse, GetTimelineByBranchRequest,
    GetTimelineByIdRequest, GetTimelineHeadRequest, GetTimelineHeadResponse, GetTimelineResponse,
    GetTimelinesForSessionRequest, GetTimelinesForSessionResponse, InsertNodeRequest,
    InsertNodeResponse, IsTimelineSealedRequest, IsTimelineSealedResponse, LatestNodeHashRequest,
    LatestNodeHashResponse, LatestStateRequest, LatestStateResponse, ListSessionsRequest,
    ListSessionsResponse, PingRequest, RenameTimelineRequest, RenameTimelineResponse,
    SealTimelineRequest, SealTimelineResponse, SemVer, SetCheckoutStateRequest,
    SetCheckoutStateResponse, UpdateTimelineHeadRequest, UpdateTimelineHeadResponse,
};
use vlinder_core::domain::DagStore;

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
            .get_node(&req.hash)
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
            .get_session_nodes(&req.session_id)
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
            .get_children(&req.parent_hash)
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
            .latest_node_hash(&req.session_id)
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

    // -------------------------------------------------------------------------
    // Timeline RPCs (ADR 093)
    // -------------------------------------------------------------------------

    async fn create_timeline(
        &self,
        request: Request<CreateTimelineRequest>,
    ) -> Result<Response<CreateTimelineResponse>, Status> {
        let req = request.into_inner();
        let id = self
            .store
            .create_timeline(
                &req.branch_name,
                &req.session_id,
                req.parent_id,
                req.fork_point.as_deref(),
            )
            .map_err(Status::internal)?;
        Ok(Response::new(CreateTimelineResponse { id }))
    }

    async fn get_timeline_by_branch(
        &self,
        request: Request<GetTimelineByBranchRequest>,
    ) -> Result<Response<GetTimelineResponse>, Status> {
        let req = request.into_inner();
        let timeline = self
            .store
            .get_timeline_by_branch(&req.branch_name)
            .map_err(Status::internal)?
            .map(|t| t.into());
        Ok(Response::new(GetTimelineResponse { timeline }))
    }

    async fn get_timeline(
        &self,
        request: Request<GetTimelineByIdRequest>,
    ) -> Result<Response<GetTimelineResponse>, Status> {
        let req = request.into_inner();
        let timeline = self
            .store
            .get_timeline(req.id)
            .map_err(Status::internal)?
            .map(|t| t.into());
        Ok(Response::new(GetTimelineResponse { timeline }))
    }

    async fn seal_timeline(
        &self,
        request: Request<SealTimelineRequest>,
    ) -> Result<Response<SealTimelineResponse>, Status> {
        let req = request.into_inner();
        match self.store.seal_timeline(req.id) {
            Ok(()) => Ok(Response::new(SealTimelineResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(SealTimelineResponse {
                success: false,
                error: Some(e),
            })),
        }
    }

    async fn rename_timeline(
        &self,
        request: Request<RenameTimelineRequest>,
    ) -> Result<Response<RenameTimelineResponse>, Status> {
        let req = request.into_inner();
        match self.store.rename_timeline(req.id, &req.new_name) {
            Ok(()) => Ok(Response::new(RenameTimelineResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(RenameTimelineResponse {
                success: false,
                error: Some(e),
            })),
        }
    }

    async fn is_timeline_sealed(
        &self,
        request: Request<IsTimelineSealedRequest>,
    ) -> Result<Response<IsTimelineSealedResponse>, Status> {
        let req = request.into_inner();
        let sealed = self
            .store
            .is_timeline_sealed(req.id)
            .map_err(Status::internal)?;
        Ok(Response::new(IsTimelineSealedResponse { sealed }))
    }

    // -------------------------------------------------------------------------
    // Session query RPCs (ADR 113)
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

    async fn get_timelines_for_session(
        &self,
        request: Request<GetTimelinesForSessionRequest>,
    ) -> Result<Response<GetTimelinesForSessionResponse>, Status> {
        let req = request.into_inner();
        let timelines = self
            .store
            .get_timelines_for_session(&req.session_id)
            .map_err(Status::internal)?
            .into_iter()
            .map(|t| t.into())
            .collect();
        Ok(Response::new(GetTimelinesForSessionResponse { timelines }))
    }

    // -------------------------------------------------------------------------
    // Timeline head RPCs (issue #37)
    // -------------------------------------------------------------------------

    async fn get_timeline_head(
        &self,
        request: Request<GetTimelineHeadRequest>,
    ) -> Result<Response<GetTimelineHeadResponse>, Status> {
        let req = request.into_inner();
        let hash = self
            .store
            .get_timeline_head(req.timeline_id)
            .map_err(Status::internal)?;
        Ok(Response::new(GetTimelineHeadResponse { hash }))
    }

    async fn update_timeline_head(
        &self,
        request: Request<UpdateTimelineHeadRequest>,
    ) -> Result<Response<UpdateTimelineHeadResponse>, Status> {
        let req = request.into_inner();
        match self.store.update_timeline_head(req.timeline_id, &req.hash) {
            Ok(()) => Ok(Response::new(UpdateTimelineHeadResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(UpdateTimelineHeadResponse {
                success: false,
                error: Some(e),
            })),
        }
    }
}
