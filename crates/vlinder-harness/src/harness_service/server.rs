//! gRPC server wrapping the Harness trait.

use std::sync::{Arc, Mutex};

use tonic::{Request, Response, Status};

use super::proto::{
    self, harness_server::Harness as HarnessService, PingRequest, RunAgentRequest,
    RunAgentResponse, SemVer, SetDagParentRequest, SetDagParentResponse, SetInitialStateRequest,
    SetInitialStateResponse, SetTimelineRequest, SetTimelineResponse, StartSessionRequest,
    StartSessionResponse,
};
use vlinder_core::domain::{Harness, ResourceId, TimelineId};

/// gRPC server that wraps a Harness implementation.
///
/// Uses `Arc<Mutex<…>>` so the harness can be shared into
/// `spawn_blocking` for long-running calls like `run_agent`.
pub struct HarnessServiceServer {
    harness: Arc<Mutex<Box<dyn Harness + Send>>>,
}

impl HarnessServiceServer {
    pub fn new(harness: Box<dyn Harness + Send>) -> Self {
        Self {
            harness: Arc::new(Mutex::new(harness)),
        }
    }

    /// Create a tonic service from this server.
    pub fn into_service(self) -> proto::harness_server::HarnessServer<Self> {
        proto::harness_server::HarnessServer::new(self)
    }
}

#[tonic::async_trait]
impl HarnessService for HarnessServiceServer {
    async fn ping(&self, _request: Request<PingRequest>) -> Result<Response<SemVer>, Status> {
        Ok(Response::new(SemVer {
            major: 0,
            minor: 0,
            patch: 1,
        }))
    }

    async fn set_timeline(
        &self,
        request: Request<SetTimelineRequest>,
    ) -> Result<Response<SetTimelineResponse>, Status> {
        let req = request.into_inner();
        let timeline = TimelineId::from(req.timeline_id);
        self.harness
            .lock()
            .unwrap()
            .set_timeline(timeline, req.sealed);
        Ok(Response::new(SetTimelineResponse {}))
    }

    async fn start_session(
        &self,
        request: Request<StartSessionRequest>,
    ) -> Result<Response<StartSessionResponse>, Status> {
        let req = request.into_inner();
        self.harness.lock().unwrap().start_session(&req.agent_name);
        Ok(Response::new(StartSessionResponse {}))
    }

    async fn set_initial_state(
        &self,
        request: Request<SetInitialStateRequest>,
    ) -> Result<Response<SetInitialStateResponse>, Status> {
        let req = request.into_inner();
        self.harness.lock().unwrap().set_initial_state(req.state);
        Ok(Response::new(SetInitialStateResponse {}))
    }

    async fn set_dag_parent(
        &self,
        request: Request<SetDagParentRequest>,
    ) -> Result<Response<SetDagParentResponse>, Status> {
        let req = request.into_inner();
        self.harness.lock().unwrap().set_dag_parent(req.hash);
        Ok(Response::new(SetDagParentResponse {}))
    }

    async fn run_agent(
        &self,
        request: Request<RunAgentRequest>,
    ) -> Result<Response<RunAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = req.agent_id.clone();
        let input = req.input.clone();
        let harness = Arc::clone(&self.harness);

        // run_agent blocks (poll loop waiting for agent completion).
        // Offload to a blocking thread so the tokio runtime stays healthy
        // for h2 connection management.
        let result = tokio::task::spawn_blocking(move || {
            let id = ResourceId::new(&agent_id);
            harness.lock().unwrap().run_agent(&id, &input)
        })
        .await
        .map_err(|e| Status::internal(format!("spawn_blocking failed: {}", e)))?;

        match result {
            Ok(output) => Ok(Response::new(RunAgentResponse {
                output,
                error: None,
            })),
            Err(e) => Ok(Response::new(RunAgentResponse {
                output: String::new(),
                error: Some(e),
            })),
        }
    }
}
