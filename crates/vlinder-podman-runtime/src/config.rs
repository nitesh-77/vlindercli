//! Configuration for the Podman container runtime.

/// Configuration for the Podman container runtime.
///
/// Extracted from vlinderd's full Config to decouple the runtime
/// crate from daemon configuration.
#[derive(Clone, Debug)]
pub struct PodmanRuntimeConfig {
    /// "mutable" or "pinned" (ADR 073)
    pub image_policy: String,
    /// "auto", "disabled", or explicit socket path (ADR 077)
    pub podman_socket: String,
    /// OCI image ref for the sidecar container
    pub sidecar_image: String,
    /// NATS URL for sidecar env vars
    pub nats_url: String,
    /// Registry gRPC address for sidecar env vars
    pub registry_addr: String,
    /// State service gRPC address for sidecar env vars
    pub state_addr: String,
    /// Secret store gRPC address for sidecar env vars
    pub secret_addr: String,
}
