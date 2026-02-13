//! Worker role definitions for distributed mode.
//!
//! When running in distributed mode, each worker process assumes a specific
//! role. The role determines which queues the worker subscribes to and what
//! processing it performs.
//!
//! Workers read their role from the VLINDER_WORKER_ROLE environment variable.

use std::fmt;
use std::str::FromStr;

/// Role that a worker process can assume.
///
/// Each role corresponds to a specific service type in the Vlinder architecture.
/// Workers subscribe to queues matching their role and process messages accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkerRole {
    /// Registry service - coordinates agents, models, and jobs
    Registry,
    /// Container agent runtime - executes OCI container agents via Podman
    AgentContainer,
    /// Ollama inference service
    InferenceOllama,
    /// OpenRouter inference service (cloud LLMs)
    InferenceOpenRouter,
    /// Ollama embedding service
    EmbeddingOllama,
    /// SQLite object storage service
    StorageObjectSqlite,
    /// In-memory object storage service
    StorageObjectMemory,
    /// SQLite-vec vector storage service
    StorageVectorSqlite,
    /// In-memory vector storage service
    StorageVectorMemory,
    /// State service — gRPC interface to the DagStore (ADR 079)
    State,
    /// DAG SQLite worker — indexes messages into Merkle DAG (SQLite)
    DagSqlite,
    /// DAG git worker — writes messages as git commits for time-travel
    DagGit,
}

impl WorkerRole {
    /// Read worker role from VLINDER_WORKER_ROLE environment variable.
    ///
    /// Returns None if the env var is not set or has an invalid value.
    pub fn from_env() -> Option<Self> {
        std::env::var("VLINDER_WORKER_ROLE")
            .ok()
            .and_then(|v| v.parse().ok())
    }

    /// Get the environment variable value for this role.
    pub fn as_env_value(&self) -> &'static str {
        match self {
            WorkerRole::Registry => "registry",
            WorkerRole::AgentContainer => "agent-container",
            WorkerRole::InferenceOllama => "inference-ollama",
            WorkerRole::InferenceOpenRouter => "inference-openrouter",
            WorkerRole::EmbeddingOllama => "embedding-ollama",
            WorkerRole::StorageObjectSqlite => "storage-object-sqlite",
            WorkerRole::StorageObjectMemory => "storage-object-memory",
            WorkerRole::StorageVectorSqlite => "storage-vector-sqlite",
            WorkerRole::StorageVectorMemory => "storage-vector-memory",
            WorkerRole::State => "state",
            WorkerRole::DagSqlite => "dag-sqlite",
            WorkerRole::DagGit => "dag-git",
        }
    }

    /// Get a human-readable description of this role.
    pub fn description(&self) -> &'static str {
        match self {
            WorkerRole::Registry => "Registry service",
            WorkerRole::AgentContainer => "Container agent runtime",
            WorkerRole::InferenceOllama => "Ollama inference service",
            WorkerRole::InferenceOpenRouter => "OpenRouter inference service",
            WorkerRole::EmbeddingOllama => "Ollama embedding service",
            WorkerRole::StorageObjectSqlite => "SQLite object storage",
            WorkerRole::StorageObjectMemory => "In-memory object storage",
            WorkerRole::StorageVectorSqlite => "SQLite-vec vector storage",
            WorkerRole::StorageVectorMemory => "In-memory vector storage",
            WorkerRole::State => "State service",
            WorkerRole::DagSqlite => "DAG SQLite worker",
            WorkerRole::DagGit => "DAG git worker",
        }
    }
}

impl fmt::Display for WorkerRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_env_value())
    }
}

impl FromStr for WorkerRole {
    type Err = ParseWorkerRoleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "registry" => Ok(WorkerRole::Registry),
            "agent-container" => Ok(WorkerRole::AgentContainer),
            "inference-ollama" => Ok(WorkerRole::InferenceOllama),
            "inference-openrouter" => Ok(WorkerRole::InferenceOpenRouter),
            "embedding-ollama" => Ok(WorkerRole::EmbeddingOllama),
            "storage-object-sqlite" => Ok(WorkerRole::StorageObjectSqlite),
            "storage-object-memory" => Ok(WorkerRole::StorageObjectMemory),
            "storage-vector-sqlite" => Ok(WorkerRole::StorageVectorSqlite),
            "storage-vector-memory" => Ok(WorkerRole::StorageVectorMemory),
            "state" => Ok(WorkerRole::State),
            "dag-sqlite" => Ok(WorkerRole::DagSqlite),
            "dag-git" => Ok(WorkerRole::DagGit),
            _ => Err(ParseWorkerRoleError(s.to_string())),
        }
    }
}

#[derive(Debug)]
pub struct ParseWorkerRoleError(String);

impl fmt::Display for ParseWorkerRoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid worker role: {}", self.0)
    }
}

impl std::error::Error for ParseWorkerRoleError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_roles() {
        assert_eq!("registry".parse::<WorkerRole>().unwrap(), WorkerRole::Registry);
        assert_eq!("agent-container".parse::<WorkerRole>().unwrap(), WorkerRole::AgentContainer);
        assert_eq!("inference-ollama".parse::<WorkerRole>().unwrap(), WorkerRole::InferenceOllama);
        assert_eq!("embedding-ollama".parse::<WorkerRole>().unwrap(), WorkerRole::EmbeddingOllama);
        assert_eq!("storage-object-sqlite".parse::<WorkerRole>().unwrap(), WorkerRole::StorageObjectSqlite);
        assert_eq!("storage-vector-sqlite".parse::<WorkerRole>().unwrap(), WorkerRole::StorageVectorSqlite);
        assert_eq!("state".parse::<WorkerRole>().unwrap(), WorkerRole::State);
        assert_eq!("dag-sqlite".parse::<WorkerRole>().unwrap(), WorkerRole::DagSqlite);
        assert_eq!("dag-git".parse::<WorkerRole>().unwrap(), WorkerRole::DagGit);
    }

    #[test]
    fn parse_invalid_role() {
        assert!("invalid".parse::<WorkerRole>().is_err());
        assert!("".parse::<WorkerRole>().is_err());
        // Old role name no longer valid
        assert!("dag-capture".parse::<WorkerRole>().is_err());
    }

    #[test]
    fn roundtrip_env_value() {
        for role in [
            WorkerRole::Registry,
            WorkerRole::AgentContainer,
            WorkerRole::InferenceOllama,
            WorkerRole::EmbeddingOllama,
            WorkerRole::StorageObjectSqlite,
            WorkerRole::StorageVectorSqlite,
            WorkerRole::State,
            WorkerRole::DagSqlite,
            WorkerRole::DagGit,
        ] {
            let env_val = role.as_env_value();
            let parsed: WorkerRole = env_val.parse().unwrap();
            assert_eq!(parsed, role);
        }
    }

    #[test]
    fn from_env_parses_valid_role() {
        // Test the parsing logic directly (avoids env var race conditions)
        let result = "agent-container".parse::<WorkerRole>();
        assert_eq!(result.unwrap(), WorkerRole::AgentContainer);
    }

    #[test]
    fn from_env_rejects_invalid_role() {
        let result = "invalid-role".parse::<WorkerRole>();
        assert!(result.is_err());
    }
}
