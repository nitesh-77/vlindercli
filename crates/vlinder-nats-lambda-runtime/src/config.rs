//! Configuration for the Lambda runtime.

/// Configuration for the Lambda runtime.
///
/// One worker = one region. The worker manages IAM roles and Lambda functions
/// for all agents assigned to `RuntimeType::Lambda`.
#[derive(Clone, Debug)]
pub struct LambdaRuntimeConfig {
    /// Registry gRPC address for agent discovery.
    pub registry_addr: String,
    /// AWS region for Lambda functions (e.g. "us-east-1").
    pub region: String,
    /// Memory allocation for Lambda functions in MB.
    pub memory_mb: i32,
    /// Execution timeout for Lambda functions in seconds.
    pub timeout_secs: i32,
}
