//! gRPC server wrapping the Harness trait.

use std::sync::Mutex;

use tonic::{Request, Response, Status};

use crate::domain::{Harness, ResourceId, TimelineId};
use super::proto::{
    self,
    harness_server::Harness as HarnessService,
    SetTimelineRequest, SetTimelineResponse,
    StartSessionRequest, StartSessionResponse,
    SetInitialStateRequest, SetInitialStateResponse,
    RunAgentRequest, RunAgentResponse,
};

/// gRPC server that wraps a Harness implementation.
pub struct HarnessServiceServer {
    harness: Mutex<Box<dyn Harness + Send>>,
}

impl HarnessServiceServer {
    pub fn new(harness: Box<dyn Harness + Send>) -> Self {
        Self {
            harness: Mutex::new(harness),
        }
    }

    /// Create a tonic service from this server.
    pub fn into_service(self) -> proto::harness_server::HarnessServer<Self> {
        proto::harness_server::HarnessServer::new(self)
    }
}

#[tonic::async_trait]
impl HarnessService for HarnessServiceServer {
    async fn set_timeline(
        &self,
        request: Request<SetTimelineRequest>,
    ) -> Result<Response<SetTimelineResponse>, Status> {
        let req = request.into_inner();
        let timeline = TimelineId::from(req.timeline_id);
        self.harness.lock().unwrap().set_timeline(timeline, req.sealed);
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

    async fn run_agent(
        &self,
        request: Request<RunAgentRequest>,
    ) -> Result<Response<RunAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = ResourceId::new(&req.agent_id);

        match self.harness.lock().unwrap().run_agent(&agent_id, &req.input) {
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
