//! Conversions between proto types and domain types for the catalog service.

use super::proto;
use vlinder_core::domain::{self, ResourceId};

// ============================================================================
// Model: domain → proto
// ============================================================================

impl From<domain::Model> for proto::Model {
    fn from(m: domain::Model) -> Self {
        Self {
            id: m.id.to_string(),
            name: m.name,
            model_type: model_type_to_proto(&m.model_type) as i32,
            provider: provider_to_proto(m.provider) as i32,
            model_path: m.model_path.to_string(),
            digest: m.digest,
        }
    }
}

// ============================================================================
// Model: proto → domain
// ============================================================================

impl TryFrom<proto::Model> for domain::Model {
    type Error = String;

    fn try_from(m: proto::Model) -> Result<Self, Self::Error> {
        Ok(Self {
            id: ResourceId::new(&m.id),
            name: m.name,
            model_type: model_type_from_proto(m.model_type)?,
            provider: provider_from_proto(m.provider)?,
            model_path: ResourceId::new(&m.model_path),
            digest: m.digest,
        })
    }
}

// ============================================================================
// ModelInfo: domain → proto
// ============================================================================

impl From<domain::ModelInfo> for proto::ModelInfo {
    fn from(info: domain::ModelInfo) -> Self {
        Self {
            name: info.name,
            size: info.size,
            modified: info.modified,
            digest: info.digest,
        }
    }
}

// ============================================================================
// ModelInfo: proto → domain
// ============================================================================

impl From<proto::ModelInfo> for domain::ModelInfo {
    fn from(info: proto::ModelInfo) -> Self {
        Self {
            name: info.name,
            size: info.size,
            modified: info.modified,
            digest: info.digest,
        }
    }
}

// ============================================================================
// Enum helpers
// ============================================================================

fn model_type_to_proto(mt: &domain::ModelType) -> proto::ModelType {
    match mt {
        domain::ModelType::Inference => proto::ModelType::Inference,
        domain::ModelType::Embedding => proto::ModelType::Embedding,
    }
}

fn model_type_from_proto(value: i32) -> Result<domain::ModelType, String> {
    match proto::ModelType::try_from(value) {
        Ok(proto::ModelType::Inference) => Ok(domain::ModelType::Inference),
        Ok(proto::ModelType::Embedding) => Ok(domain::ModelType::Embedding),
        Ok(proto::ModelType::Unspecified) | Err(_) => Err(format!("invalid model type: {value}")),
    }
}

fn provider_to_proto(p: domain::Provider) -> proto::Provider {
    match p {
        domain::Provider::Ollama => proto::Provider::Ollama,
        domain::Provider::OpenRouter => proto::Provider::Openrouter,
    }
}

fn provider_from_proto(value: i32) -> Result<domain::Provider, String> {
    match proto::Provider::try_from(value) {
        Ok(proto::Provider::Ollama) => Ok(domain::Provider::Ollama),
        Ok(proto::Provider::Openrouter) => Ok(domain::Provider::OpenRouter),
        Ok(proto::Provider::Unspecified) | Err(_) => Err(format!("invalid provider: {value}")),
    }
}
