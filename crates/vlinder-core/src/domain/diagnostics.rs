//! Diagnostics types for message observability (ADR 071).
//!
//! Each message type carries diagnostics specific to its emitter.
//! The type system encodes what each platform component guarantees —
//! no shared `Diagnostics` struct with optional fields.
//!
//! | Message   | Diagnostics type       | Emitter                  |
//! |-----------|------------------------|--------------------------|
//! | Invoke    | InvokeDiagnostics      | Harness                  |
//! | Request   | RequestDiagnostics     | Provider server        |
//! | Response  | ServiceDiagnostics     | Service workers          |
//! | Complete  | ContainerDiagnostics   | Container runtime        |
//! | Delegate  | DelegateDiagnostics    | Container runtime        |

use serde::{Deserialize, Serialize};

use super::container_id::ContainerId;
use super::image_digest::ImageDigest;
use super::image_ref::ImageRef;
use super::operation::Operation;
use super::service_type::ServiceType;

// ============================================================================
// InvokeDiagnostics — Harness
// ============================================================================

/// Diagnostics emitted by the harness when creating an InvokeMessage.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InvokeDiagnostics {
    /// Harness version (e.g., `env!("CARGO_PKG_VERSION")`).
    pub harness_version: String,
    /// Number of history turns included in the enriched payload.
    pub history_turns: u32,
}

// ============================================================================
// RequestDiagnostics — Provider server
// ============================================================================

/// Diagnostics emitted by the bridge when intercepting an agent SDK call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RequestDiagnostics {
    /// Sequence number within the submission.
    pub sequence: u32,
    /// The bridge endpoint the agent called (e.g., "/infer", "/kv/get").
    pub endpoint: String,
    /// Size of the agent's request body in bytes.
    pub request_bytes: u64,
    /// Timestamp when the bridge received the call (Unix millis).
    pub received_at_ms: u64,
}

// ============================================================================
// ServiceDiagnostics — Service workers (Response)
// ============================================================================

/// Diagnostics emitted by a service worker after processing a request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ServiceDiagnostics {
    /// Which platform service handled this request.
    pub service: ServiceType,
    /// Backend identifier: "ollama", "openrouter", "sqlite", "memory".
    pub backend: String,
    /// Execution time in milliseconds.
    pub duration_ms: u64,
    /// Service-specific metrics.
    pub metrics: ServiceMetrics,
}

/// Service-specific metrics — the type system enforces which metrics
/// each service kind reports.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ServiceMetrics {
    Inference {
        tokens_input: u32,
        tokens_output: u32,
        model: String,
    },
    Embedding {
        dimensions: u32,
        model: String,
    },
    Storage {
        operation: Operation,
        bytes_transferred: u64,
    },
}

impl ServiceDiagnostics {
    /// Placeholder diagnostics for when real service metadata is not available.
    ///
    /// Used during NATS deserialization as a fallback for messages from
    /// older senders that don't yet include diagnostics headers.
    pub fn placeholder() -> Self {
        Self {
            service: ServiceType::Kv,
            backend: "unknown".to_string(),
            duration_ms: 0,
            metrics: ServiceMetrics::Storage {
                operation: Operation::Get,
                bytes_transferred: 0,
            },
        }
    }

    /// Convenience constructor for storage service workers (kv, vec).
    pub fn storage(
        service: ServiceType,
        backend: impl Into<String>,
        operation: Operation,
        bytes: u64,
        duration_ms: u64,
    ) -> Self {
        Self {
            service,
            backend: backend.into(),
            duration_ms,
            metrics: ServiceMetrics::Storage {
                operation,
                bytes_transferred: bytes,
            },
        }
    }
}

// ============================================================================
// ContainerDiagnostics — Container runtime (Complete)
// ============================================================================

/// Diagnostics emitted by the container runtime on task completion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContainerDiagnostics {
    /// Agent's stderr stream, extracted from the HTTP response.
    /// Stored as a separate binary file in the git tree, not in diagnostics.toml.
    #[serde(skip)]
    pub stderr: Vec<u8>,
    /// Container runtime metadata.
    pub runtime: ContainerRuntimeInfo,
    /// Wall-clock execution time in milliseconds.
    pub duration_ms: u64,
}

/// Container runtime metadata — populated entirely by the platform.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContainerRuntimeInfo {
    /// Podman engine version (e.g., "5.3.1").
    pub engine_version: String,
    /// OCI image reference (e.g., "localhost/echo-agent:latest").
    pub image_ref: Option<ImageRef>,
    /// Image digest (e.g., "sha256:a80c4f17..."), if resolved.
    pub image_digest: Option<ImageDigest>,
    /// Container ID for this execution.
    pub container_id: ContainerId,
}

impl ContainerDiagnostics {
    /// Placeholder diagnostics for when real container metadata is not yet available.
    ///
    /// Stderr and Podman metadata are deferred — this provides compile-time
    /// completeness while the container integration catches up.
    pub fn placeholder(duration_ms: u64) -> Self {
        Self {
            stderr: Vec::new(),
            runtime: ContainerRuntimeInfo {
                engine_version: "unknown".to_string(),
                image_ref: None,
                image_digest: None,
                container_id: ContainerId::unknown(),
            },
            duration_ms,
        }
    }
}

// ============================================================================
// DelegateDiagnostics — Container runtime (Delegate)
// ============================================================================

/// Diagnostics emitted when an agent delegates to another agent.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DelegateDiagnostics {
    /// Delegation involves container execution — same diagnostics.
    pub container: ContainerDiagnostics,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_diagnostics_json_round_trip() {
        let diag = InvokeDiagnostics {
            harness_version: "0.1.0".to_string(),
            history_turns: 3,
        };
        let json = serde_json::to_string(&diag).unwrap();
        let back: InvokeDiagnostics = serde_json::from_str(&json).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn invoke_diagnostics_toml_round_trip() {
        let diag = InvokeDiagnostics {
            harness_version: "0.1.0".to_string(),
            history_turns: 3,
        };
        let toml_str = toml::to_string_pretty(&diag).unwrap();
        let back: InvokeDiagnostics = toml::from_str(&toml_str).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn request_diagnostics_json_round_trip() {
        let diag = RequestDiagnostics {
            sequence: 1,
            endpoint: "/infer".to_string(),
            request_bytes: 1024,
            received_at_ms: 1700000000000,
        };
        let json = serde_json::to_string(&diag).unwrap();
        let back: RequestDiagnostics = serde_json::from_str(&json).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn request_diagnostics_toml_round_trip() {
        let diag = RequestDiagnostics {
            sequence: 1,
            endpoint: "/infer".to_string(),
            request_bytes: 1024,
            received_at_ms: 1700000000000,
        };
        let toml_str = toml::to_string_pretty(&diag).unwrap();
        let back: RequestDiagnostics = toml::from_str(&toml_str).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn service_diagnostics_inference_json_round_trip() {
        let diag = ServiceDiagnostics {
            service: ServiceType::Infer,
            backend: "ollama".to_string(),
            duration_ms: 1800,
            metrics: ServiceMetrics::Inference {
                tokens_input: 512,
                tokens_output: 908,
                model: "phi3:latest".to_string(),
            },
        };
        let json = serde_json::to_string(&diag).unwrap();
        let back: ServiceDiagnostics = serde_json::from_str(&json).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn service_diagnostics_inference_toml_round_trip() {
        let diag = ServiceDiagnostics {
            service: ServiceType::Infer,
            backend: "ollama".to_string(),
            duration_ms: 1800,
            metrics: ServiceMetrics::Inference {
                tokens_input: 512,
                tokens_output: 908,
                model: "phi3:latest".to_string(),
            },
        };
        let toml_str = toml::to_string_pretty(&diag).unwrap();
        let back: ServiceDiagnostics = toml::from_str(&toml_str).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn service_diagnostics_embedding_json_round_trip() {
        let diag = ServiceDiagnostics {
            service: ServiceType::Embed,
            backend: "ollama".to_string(),
            duration_ms: 200,
            metrics: ServiceMetrics::Embedding {
                dimensions: 768,
                model: "nomic-embed-text".to_string(),
            },
        };
        let json = serde_json::to_string(&diag).unwrap();
        let back: ServiceDiagnostics = serde_json::from_str(&json).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn service_diagnostics_storage_json_round_trip() {
        let diag = ServiceDiagnostics::storage(ServiceType::Kv, "sqlite", Operation::Put, 2048, 5);
        let json = serde_json::to_string(&diag).unwrap();
        let back: ServiceDiagnostics = serde_json::from_str(&json).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn service_diagnostics_storage_toml_round_trip() {
        let diag = ServiceDiagnostics::storage(ServiceType::Kv, "sqlite", Operation::Get, 512, 2);
        let toml_str = toml::to_string_pretty(&diag).unwrap();
        let back: ServiceDiagnostics = toml::from_str(&toml_str).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn container_diagnostics_json_round_trip() {
        // stderr is #[serde(skip)] — stored as a separate binary blob (ADR 078).
        // Round-trip only preserves the non-skipped fields.
        let diag = ContainerDiagnostics {
            stderr: b"INFO: loaded model".to_vec(),
            runtime: ContainerRuntimeInfo {
                engine_version: "5.3.1".to_string(),
                image_ref: Some(ImageRef::parse("localhost/echo-agent:latest").unwrap()),
                image_digest: Some(ImageDigest::parse("sha256:abc123").unwrap()),
                container_id: ContainerId::new("def456"),
            },
            duration_ms: 2300,
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(!json.contains("stderr"), "stderr should not appear in JSON");
        let back: ContainerDiagnostics = serde_json::from_str(&json).unwrap();
        assert!(back.stderr.is_empty(), "stderr defaults to empty on deserialize");
        assert_eq!(back.runtime, diag.runtime);
        assert_eq!(back.duration_ms, diag.duration_ms);
    }

    #[test]
    fn container_diagnostics_toml_round_trip() {
        // stderr is #[serde(skip)] — stored as a separate binary blob (ADR 078).
        let diag = ContainerDiagnostics {
            stderr: b"WARN: truncated".to_vec(),
            runtime: ContainerRuntimeInfo {
                engine_version: "5.3.1".to_string(),
                image_ref: Some(ImageRef::parse("localhost/support-agent:latest").unwrap()),
                image_digest: Some(ImageDigest::parse("sha256:a80c4f17").unwrap()),
                container_id: ContainerId::new("abc123def456"),
            },
            duration_ms: 2300,
        };
        let toml_str = toml::to_string_pretty(&diag).unwrap();
        assert!(!toml_str.contains("stderr"), "stderr should not appear in TOML");
        let back: ContainerDiagnostics = toml::from_str(&toml_str).unwrap();
        assert!(back.stderr.is_empty(), "stderr defaults to empty on deserialize");
        assert_eq!(back.runtime, diag.runtime);
        assert_eq!(back.duration_ms, diag.duration_ms);
    }

    #[test]
    fn container_diagnostics_placeholder() {
        let diag = ContainerDiagnostics::placeholder(100);
        assert!(diag.stderr.is_empty());
        assert_eq!(diag.runtime.engine_version, "unknown");
        assert_eq!(diag.duration_ms, 100);
    }

    #[test]
    fn delegate_diagnostics_json_round_trip() {
        let diag = DelegateDiagnostics {
            container: ContainerDiagnostics::placeholder(50),
        };
        let json = serde_json::to_string(&diag).unwrap();
        let back: DelegateDiagnostics = serde_json::from_str(&json).unwrap();
        assert_eq!(diag, back);
    }

    #[test]
    fn delegate_diagnostics_toml_round_trip() {
        let diag = DelegateDiagnostics {
            container: ContainerDiagnostics::placeholder(50),
        };
        let toml_str = toml::to_string_pretty(&diag).unwrap();
        let back: DelegateDiagnostics = toml::from_str(&toml_str).unwrap();
        assert_eq!(diag, back);
    }
}
