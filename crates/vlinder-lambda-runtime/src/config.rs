//! Configuration for the Lambda runtime.

/// Configuration for the Lambda runtime.
///
/// Minimal for the spike — just needs registry connectivity.
/// Future: AWS region, role ARN, function naming convention, etc.
#[derive(Clone, Debug)]
pub struct LambdaRuntimeConfig {
    /// Registry gRPC address for agent discovery
    pub registry_addr: String,
}
