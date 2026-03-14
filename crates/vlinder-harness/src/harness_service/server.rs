//! gRPC server wrapping the Harness trait.

use std::sync::{Arc, Mutex};

use tonic::{Request, Response, Status};

use super::proto::{
    self, harness_server::Harness as HarnessService, ForkTimelineRequest, ForkTimelineResponse,
    PingRequest, RepairAgentRequest, RepairAgentResponse, RunAgentRequest, RunAgentResponse,
    SemVer, SetDagParentRequest, SetDagParentResponse, SetInitialStateRequest,
    SetInitialStateResponse, SetTimelineRequest, SetTimelineResponse, StartSessionRequest,
    StartSessionResponse,
};
use std::str::FromStr;
use vlinder_core::domain::{
    AgentId, DagNodeId, ForkParams, Harness, Operation, RepairParams, ResourceId, Sequence,
    ServiceBackend, ServiceType, TimelineId,
};

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
        let harness = Arc::clone(&self.harness);
        tokio::task::spawn_blocking(move || {
            harness.lock().unwrap().start_session(&req.agent_name);
        })
        .await
        .map_err(|e| Status::internal(format!("spawn_blocking failed: {}", e)))?;
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
        self.harness
            .lock()
            .unwrap()
            .set_dag_parent(DagNodeId::from(req.hash));
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

    async fn repair_agent(
        &self,
        request: Request<RepairAgentRequest>,
    ) -> Result<Response<RepairAgentResponse>, Status> {
        let req = request.into_inner();
        let harness = Arc::clone(&self.harness);

        let result = tokio::task::spawn_blocking(move || {
            let service_type = ServiceType::from_str(&req.service)
                .map_err(|_| format!("unknown service type: {}", req.service))?;
            let service =
                ServiceBackend::from_parts(service_type, &req.backend).ok_or_else(|| {
                    format!("invalid service/backend: {}/{}", req.service, req.backend)
                })?;
            let operation = Operation::from_str(&req.operation)
                .map_err(|_| format!("unknown operation: {}", req.operation))?;

            let params = RepairParams {
                agent_id: AgentId::new(&req.agent_id),
                dag_parent: DagNodeId::from(req.dag_parent),
                checkpoint: req.checkpoint,
                service,
                operation,
                sequence: Sequence::from(req.sequence),
                payload: req.payload,
                state: req.state,
            };

            harness.lock().unwrap().repair_agent(params)
        })
        .await
        .map_err(|e| Status::internal(format!("spawn_blocking failed: {}", e)))?;

        match result {
            Ok(output) => Ok(Response::new(RepairAgentResponse {
                output,
                error: None,
            })),
            Err(e) => Ok(Response::new(RepairAgentResponse {
                output: String::new(),
                error: Some(e),
            })),
        }
    }

    async fn fork_timeline(
        &self,
        request: Request<ForkTimelineRequest>,
    ) -> Result<Response<ForkTimelineResponse>, Status> {
        let req = request.into_inner();
        let harness = Arc::clone(&self.harness);

        let result = tokio::task::spawn_blocking(move || {
            let params = ForkParams {
                agent_name: req.agent_name,
                branch_name: req.branch_name,
                fork_point: DagNodeId::from(req.fork_point),
                parent_timeline_id: req.parent_timeline_id,
            };
            harness.lock().unwrap().fork_timeline(params)
        })
        .await
        .map_err(|e| Status::internal(format!("spawn_blocking failed: {}", e)))?;

        match result {
            Ok(()) => Ok(Response::new(ForkTimelineResponse { error: None })),
            Err(e) => Ok(Response::new(ForkTimelineResponse { error: Some(e) })),
        }
    }
}
