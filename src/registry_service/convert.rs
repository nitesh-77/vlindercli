//! Conversions between domain types and protobuf types.

use std::collections::HashMap;

use crate::domain::{
    Agent, EngineType, Job, JobId, JobStatus, Model, ModelType, Requirements, ResourceId,
};
use crate::queue::SubmissionId;
use super::proto;

// =============================================================================
// ResourceId
// =============================================================================

impl From<ResourceId> for proto::ResourceId {
    fn from(id: ResourceId) -> Self {
        Self { uri: id.to_string() }
    }
}

impl From<proto::ResourceId> for ResourceId {
    fn from(id: proto::ResourceId) -> Self {
        ResourceId::new(&id.uri)
    }
}

impl From<&ResourceId> for proto::ResourceId {
    fn from(id: &ResourceId) -> Self {
        Self { uri: id.to_string() }
    }
}

// =============================================================================
// JobId
// =============================================================================

impl From<JobId> for proto::JobId {
    fn from(id: JobId) -> Self {
        Self { id: id.as_str().to_string() }
    }
}

impl From<proto::JobId> for JobId {
    fn from(id: proto::JobId) -> Self {
        JobId::from_string(id.id)
    }
}

impl From<&JobId> for proto::JobId {
    fn from(id: &JobId) -> Self {
        Self { id: id.as_str().to_string() }
    }
}

// =============================================================================
// SubmissionId (ADR 044)
// =============================================================================

impl From<SubmissionId> for proto::SubmissionId {
    fn from(id: SubmissionId) -> Self {
        Self { id: id.as_str().to_string() }
    }
}

impl From<proto::SubmissionId> for SubmissionId {
    fn from(id: proto::SubmissionId) -> Self {
        SubmissionId::from(id.id)
    }
}

impl From<&SubmissionId> for proto::SubmissionId {
    fn from(id: &SubmissionId) -> Self {
        Self { id: id.as_str().to_string() }
    }
}

// =============================================================================
// Agent
// =============================================================================

impl From<Agent> for proto::Agent {
    fn from(agent: Agent) -> Self {
        Self {
            name: agent.name,
            description: agent.description,
            id: Some(agent.id.into()),
            required_services: agent.requirements.services,
            models: agent.requirements.models.into_iter()
                .map(|(k, v)| (k, v.to_string()))
                .collect(),
            object_storage: agent.object_storage.map(|r| proto::ObjectStorageConfig {
                resource_id: Some(r.into()),
            }),
            vector_storage: agent.vector_storage.map(|r| proto::VectorStorageConfig {
                resource_id: Some(r.into()),
                dimensions: 0, // Not stored in domain model
            }),
        }
    }
}

impl TryFrom<proto::Agent> for Agent {
    type Error = String;

    fn try_from(agent: proto::Agent) -> Result<Self, Self::Error> {
        let models: HashMap<String, ResourceId> = agent.models.into_iter()
            .map(|(k, v)| (k, ResourceId::new(&v)))
            .collect();

        Ok(Self {
            name: agent.name,
            description: agent.description,
            id: agent.id.ok_or("missing agent id")?.into(),
            requirements: Requirements {
                models,
                services: agent.required_services,
            },
            object_storage: agent.object_storage
                .and_then(|cfg| cfg.resource_id)
                .map(|r| r.into()),
            vector_storage: agent.vector_storage
                .and_then(|cfg| cfg.resource_id)
                .map(|r| r.into()),
            mounts: vec![], // Mounts not sent over gRPC
            source: None,
            prompts: None,
        })
    }
}

// =============================================================================
// Model
// =============================================================================

impl From<Model> for proto::Model {
    fn from(model: Model) -> Self {
        Self {
            id: Some(model.id.into()),
            name: model.name,
            model_type: proto::ModelType::from(model.model_type).into(),
            engine: proto::EngineType::from(model.engine).into(),
            model_path: Some(model.model_path.into()),
            digest: model.digest,
        }
    }
}

impl TryFrom<proto::Model> for Model {
    type Error = String;

    fn try_from(model: proto::Model) -> Result<Self, Self::Error> {
        Ok(Self {
            id: model.id.ok_or("missing model id")?.into(),
            name: model.name,
            model_type: proto::ModelType::try_from(model.model_type)
                .map_err(|_| "invalid model type")?
                .into(),
            engine: proto::EngineType::try_from(model.engine)
                .map_err(|_| "invalid engine type")?
                .into(),
            model_path: model.model_path.ok_or("missing model path")?.into(),
            digest: model.digest,
        })
    }
}

impl From<ModelType> for proto::ModelType {
    fn from(t: ModelType) -> Self {
        match t {
            ModelType::Inference => proto::ModelType::Inference,
            ModelType::Embedding => proto::ModelType::Embedding,
        }
    }
}

impl From<proto::ModelType> for ModelType {
    fn from(t: proto::ModelType) -> Self {
        match t {
            proto::ModelType::Inference => ModelType::Inference,
            proto::ModelType::Embedding => ModelType::Embedding,
            proto::ModelType::Unspecified => ModelType::Inference, // Default
        }
    }
}

impl From<EngineType> for proto::EngineType {
    fn from(t: EngineType) -> Self {
        match t {
            EngineType::Llama => proto::EngineType::Llama,
            EngineType::Ollama => proto::EngineType::Ollama,
            EngineType::InMemory => proto::EngineType::InMemory,
        }
    }
}

impl From<proto::EngineType> for EngineType {
    fn from(t: proto::EngineType) -> Self {
        match t {
            proto::EngineType::Llama => EngineType::Llama,
            proto::EngineType::Ollama => EngineType::Ollama,
            proto::EngineType::InMemory => EngineType::InMemory,
            proto::EngineType::Unspecified => EngineType::InMemory, // Default
        }
    }
}

// =============================================================================
// Job
// =============================================================================

impl From<Job> for proto::Job {
    fn from(job: Job) -> Self {
        let (status, output) = match job.status {
            JobStatus::Pending => (proto::JobStatus::Pending, None),
            JobStatus::Running => (proto::JobStatus::Running, None),
            JobStatus::Completed(s) => (proto::JobStatus::Completed, Some(s)),
            JobStatus::Failed(s) => (proto::JobStatus::Failed, Some(s)),
        };

        Self {
            id: Some(job.id.into()),
            submission_id: Some(job.submission_id.into()),
            agent_id: Some(job.agent_id.into()),
            input: job.input,
            status: status.into(),
            output,
        }
    }
}

impl TryFrom<proto::Job> for Job {
    type Error = String;

    fn try_from(job: proto::Job) -> Result<Self, Self::Error> {
        let proto_status = proto::JobStatus::try_from(job.status)
            .map_err(|_| "invalid job status")?;

        let status = match proto_status {
            proto::JobStatus::Pending => JobStatus::Pending,
            proto::JobStatus::Running => JobStatus::Running,
            proto::JobStatus::Completed => JobStatus::Completed(job.output.unwrap_or_default()),
            proto::JobStatus::Failed => JobStatus::Failed(job.output.unwrap_or_default()),
            proto::JobStatus::Unspecified => JobStatus::Pending,
        };

        Ok(Self {
            id: job.id.ok_or("missing job id")?.into(),
            submission_id: job.submission_id.ok_or("missing submission id")?.into(),
            agent_id: job.agent_id.ok_or("missing agent id")?.into(),
            input: job.input,
            status,
        })
    }
}

impl From<JobStatus> for proto::JobStatus {
    fn from(s: JobStatus) -> Self {
        match s {
            JobStatus::Pending => proto::JobStatus::Pending,
            JobStatus::Running => proto::JobStatus::Running,
            JobStatus::Completed(_) => proto::JobStatus::Completed,
            JobStatus::Failed(_) => proto::JobStatus::Failed,
        }
    }
}

impl From<proto::JobStatus> for JobStatus {
    fn from(s: proto::JobStatus) -> Self {
        match s {
            proto::JobStatus::Pending => JobStatus::Pending,
            proto::JobStatus::Running => JobStatus::Running,
            proto::JobStatus::Completed => JobStatus::Completed(String::new()),
            proto::JobStatus::Failed => JobStatus::Failed(String::new()),
            proto::JobStatus::Unspecified => JobStatus::Pending,
        }
    }
}
