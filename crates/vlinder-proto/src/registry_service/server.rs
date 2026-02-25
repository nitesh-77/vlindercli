//! gRPC server wrapping the Registry trait.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use vlinder_core::domain::{Registry, JobStatus as DomainJobStatus, ResourceId, SubmissionId};
use super::proto::{
    self,
    registry_server::Registry as RegistryService,
    PingRequest, SemVer,
    GetAgentRequest, GetAgentResponse, GetAgentByNameRequest,
    RegisterAgentRequest, RegisterAgentResponse,
    ListAgentsRequest, ListAgentsResponse,
    RegisterFleetRequest, RegisterFleetResponse,
    GetFleetRequest, GetFleetResponse,
    ListFleetsRequest, ListFleetsResponse,
    GetModelRequest, GetModelResponse,
    ListModelsRequest, ListModelsResponse,
    RegisterModelRequest, RegisterModelResponse,
    DeleteModelRequest, DeleteModelResponse,
    CreateJobRequest, CreateJobResponse,
    GetJobRequest, GetJobResponse,
    UpdateJobStatusRequest, UpdateJobStatusResponse,
    ListPendingJobsRequest, ListPendingJobsResponse,
};

/// gRPC server that wraps a Registry implementation.
pub struct RegistryServiceServer {
    registry: Arc<dyn Registry>,
}

impl RegistryServiceServer {
    pub fn new(registry: Arc<dyn Registry>) -> Self {
        Self { registry }
    }

    /// Create a tonic service from this server.
    pub fn into_service(self) -> proto::registry_server::RegistryServer<Self> {
        proto::registry_server::RegistryServer::new(self)
    }
}

#[tonic::async_trait]
impl RegistryService for RegistryServiceServer {
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

    async fn get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<GetAgentResponse>, Status> {
        let req = request.into_inner();
        let id: ResourceId = req.id
            .ok_or_else(|| Status::invalid_argument("missing agent id"))?
            .into();

        let agent = self.registry.get_agent(&id).map(|a| a.into());

        Ok(Response::new(GetAgentResponse { agent }))
    }

    async fn get_agent_by_name(
        &self,
        request: Request<GetAgentByNameRequest>,
    ) -> Result<Response<GetAgentResponse>, Status> {
        let req = request.into_inner();

        let agent = self.registry.get_agent_by_name(&req.name)
            .map(|a| a.into());

        Ok(Response::new(GetAgentResponse { agent }))
    }

    async fn register_agent(
        &self,
        request: Request<RegisterAgentRequest>,
    ) -> Result<Response<RegisterAgentResponse>, Status> {
        let req = request.into_inner();
        let agent = req.agent
            .ok_or_else(|| Status::invalid_argument("missing agent"))?;

        let domain_agent = agent.try_into()
            .map_err(|e: String| Status::invalid_argument(e))?;

        // TECH DEBT: spawn_blocking works around nested tokio runtime panic.
        // NatsSecretStore owns its own Runtime and calls block_on() — panics
        // when called from a tokio worker thread (this gRPC handler).
        // Real fix: secret store should be a separate process, and/or the
        // Registry trait should be async. See TODO.md.
        let registry = Arc::clone(&self.registry);
        let result = tokio::task::spawn_blocking(move || {
            registry.register_agent(domain_agent)
        }).await.map_err(|e| Status::internal(format!("task join error: {}", e)))?;

        match result {
            Ok(()) => Ok(Response::new(RegisterAgentResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(RegisterAgentResponse {
                success: false,
                error: Some(e.to_string()),
            })),
        }
    }

    async fn list_agents(
        &self,
        _request: Request<ListAgentsRequest>,
    ) -> Result<Response<ListAgentsResponse>, Status> {
        let agents = self.registry.get_agents()
            .into_iter()
            .map(|a| a.into())
            .collect();

        Ok(Response::new(ListAgentsResponse { agents }))
    }

    async fn register_fleet(
        &self,
        request: Request<RegisterFleetRequest>,
    ) -> Result<Response<RegisterFleetResponse>, Status> {
        let req = request.into_inner();
        let fleet = req.fleet
            .ok_or_else(|| Status::invalid_argument("missing fleet"))?;

        let domain_fleet = fleet.try_into()
            .map_err(|e: String| Status::invalid_argument(e))?;

        match self.registry.register_fleet(domain_fleet) {
            Ok(()) => Ok(Response::new(RegisterFleetResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(RegisterFleetResponse {
                success: false,
                error: Some(e.to_string()),
            })),
        }
    }

    async fn get_fleet(
        &self,
        request: Request<GetFleetRequest>,
    ) -> Result<Response<GetFleetResponse>, Status> {
        let req = request.into_inner();
        let fleet = self.registry.get_fleet(&req.name).map(|f| f.into());

        Ok(Response::new(GetFleetResponse { fleet }))
    }

    async fn list_fleets(
        &self,
        _request: Request<ListFleetsRequest>,
    ) -> Result<Response<ListFleetsResponse>, Status> {
        let fleets = self.registry.get_fleets()
            .into_iter()
            .map(|f| f.into())
            .collect();

        Ok(Response::new(ListFleetsResponse { fleets }))
    }

    async fn get_model(
        &self,
        request: Request<GetModelRequest>,
    ) -> Result<Response<GetModelResponse>, Status> {
        let req = request.into_inner();
        let model = self.registry.get_model(&req.name).map(|m| m.into());

        Ok(Response::new(GetModelResponse { model }))
    }

    async fn list_models(
        &self,
        _request: Request<ListModelsRequest>,
    ) -> Result<Response<ListModelsResponse>, Status> {
        let models = self.registry.get_models()
            .into_iter()
            .map(|m| m.into())
            .collect();

        Ok(Response::new(ListModelsResponse { models }))
    }

    async fn register_model(
        &self,
        request: Request<RegisterModelRequest>,
    ) -> Result<Response<RegisterModelResponse>, Status> {
        let req = request.into_inner();
        let model = req.model
            .ok_or_else(|| Status::invalid_argument("missing model"))?;

        let domain_model = model.try_into()
            .map_err(|e: String| Status::invalid_argument(e))?;

        match self.registry.register_model(domain_model) {
            Ok(()) => Ok(Response::new(RegisterModelResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(RegisterModelResponse {
                success: false,
                error: Some(e.to_string()),
            })),
        }
    }

    async fn delete_model(
        &self,
        request: Request<DeleteModelRequest>,
    ) -> Result<Response<DeleteModelResponse>, Status> {
        let req = request.into_inner();

        match self.registry.delete_model(&req.name) {
            Ok(deleted) => Ok(Response::new(DeleteModelResponse {
                deleted,
                error: None,
            })),
            Err(e) => Ok(Response::new(DeleteModelResponse {
                deleted: false,
                error: Some(e.to_string()),
            })),
        }
    }

    async fn create_job(
        &self,
        request: Request<CreateJobRequest>,
    ) -> Result<Response<CreateJobResponse>, Status> {
        let req = request.into_inner();
        let submission_id: SubmissionId = req.submission_id
            .ok_or_else(|| Status::invalid_argument("missing submission_id"))?
            .into();
        let agent_id: ResourceId = req.agent_id
            .ok_or_else(|| Status::invalid_argument("missing agent_id"))?
            .into();

        let job_id = self.registry.create_job(submission_id.clone(), agent_id, req.input);

        Ok(Response::new(CreateJobResponse {
            job_id: Some(job_id.into()),
            submission_id: Some(submission_id.into()),
        }))
    }

    async fn get_job(
        &self,
        request: Request<GetJobRequest>,
    ) -> Result<Response<GetJobResponse>, Status> {
        let req = request.into_inner();
        let job_id = req.id
            .ok_or_else(|| Status::invalid_argument("missing job id"))?
            .into();

        let job = self.registry.get_job(&job_id).map(|j| j.into());

        Ok(Response::new(GetJobResponse { job }))
    }

    async fn update_job_status(
        &self,
        request: Request<UpdateJobStatusRequest>,
    ) -> Result<Response<UpdateJobStatusResponse>, Status> {
        let req = request.into_inner();
        let job_id = req.id
            .ok_or_else(|| Status::invalid_argument("missing job id"))?
            .into();

        // Convert proto status to domain status, including output if provided
        let status: DomainJobStatus = match (proto::JobStatus::try_from(req.status), req.output) {
            (Ok(proto::JobStatus::Completed), Some(output)) => DomainJobStatus::Completed(output),
            (Ok(proto::JobStatus::Failed), Some(error)) => DomainJobStatus::Failed(error),
            (Ok(s), _) => s.into(),
            (Err(_), _) => return Err(Status::invalid_argument("invalid status")),
        };

        self.registry.update_job_status(&job_id, status);

        Ok(Response::new(UpdateJobStatusResponse { success: true }))
    }

    async fn list_pending_jobs(
        &self,
        _request: Request<ListPendingJobsRequest>,
    ) -> Result<Response<ListPendingJobsResponse>, Status> {
        let jobs = self.registry.pending_jobs()
            .into_iter()
            .map(|j| j.into())
            .collect();

        Ok(Response::new(ListPendingJobsResponse { jobs }))
    }
}
