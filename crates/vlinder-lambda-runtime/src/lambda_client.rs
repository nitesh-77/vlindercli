//! Lambda client abstraction — trait + AWS SDK implementation.
//!
//! Mirrors `podman_client.rs`: an object-safe trait for testability,
//! with `AwsLambdaClient` doing the real AWS work.

use std::fmt;

// ── Error type ──────────────────────────────────────────────────────

/// Lambda operation failure.
#[derive(Debug)]
pub enum LambdaError {
    /// Any AWS SDK or API error.
    Aws(String),
}

impl fmt::Display for LambdaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LambdaError::Aws(msg) => write!(f, "{}", msg),
        }
    }
}

// ── Function info ───────────────────────────────────────────────────

/// Minimal info about a deployed Lambda function.
#[derive(Debug, Clone)]
pub(crate) struct FunctionInfo {
    pub function_arn: String,
}

// ── Trait ────────────────────────────────────────────────────────────

/// Client abstraction over AWS Lambda + IAM.
///
/// Object-safe so `LambdaRuntime` can hold a `Box<dyn LambdaClient>`.
#[allow(dead_code)]
pub(crate) trait LambdaClient: Send {
    /// Health check: can we reach the Lambda API?
    fn check_connectivity(&self) -> Result<(), LambdaError>;

    /// Create an IAM role for a Lambda function.
    /// Returns the role ARN. Idempotent: returns existing ARN if role exists.
    fn create_role(&self, role_name: &str) -> Result<String, LambdaError>;

    /// Delete an IAM role. Fire-and-forget: errors are logged, not returned.
    fn delete_role(&self, role_name: &str);

    /// Create a Lambda function from an ECR image.
    /// Returns the function ARN. Idempotent: returns existing ARN if function exists.
    fn create_function(
        &self,
        function_name: &str,
        ecr_image_uri: &str,
        role_arn: &str,
        memory_mb: i32,
        timeout_secs: i32,
        env_vars: &[(&str, &str)],
    ) -> Result<String, LambdaError>;

    /// Get info about a Lambda function, or None if it doesn't exist.
    fn get_function(&self, function_name: &str) -> Result<Option<FunctionInfo>, LambdaError>;

    /// Delete a Lambda function. Fire-and-forget: errors are logged, not returned.
    fn delete_function(&self, function_name: &str);
}

// ── AWS SDK implementation ──────────────────────────────────────────

/// Real AWS Lambda + IAM client backed by the AWS SDK.
///
/// Owns a tokio runtime to bridge the sync `LambdaClient` trait with
/// the async AWS SDK. Each method does `self.rt.block_on(async { ... })`.
pub(crate) struct AwsLambdaClient {
    rt: tokio::runtime::Runtime,
    lambda: aws_sdk_lambda::Client,
    iam: aws_sdk_iam::Client,
}

/// Trust policy that allows Lambda to assume the role.
/// The role itself has zero permissions — the sidecar mediates all I/O.
const LAMBDA_TRUST_POLICY: &str = r#"{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": { "Service": "lambda.amazonaws.com" },
      "Action": "sts:AssumeRole"
    }
  ]
}"#;

impl AwsLambdaClient {
    /// Create a new AWS client for the given region.
    ///
    /// Uses the default credential chain (~/.aws/credentials, env vars, etc).
    pub fn new(region: &str) -> Result<Self, LambdaError> {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| LambdaError::Aws(format!("failed to create tokio runtime: {}", e)))?;

        let (lambda, iam) = rt.block_on(async {
            let region = aws_config::Region::new(region.to_string());
            let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(region)
                .load()
                .await;

            let lambda = aws_sdk_lambda::Client::new(&config);
            let iam = aws_sdk_iam::Client::new(&config);
            (lambda, iam)
        });

        Ok(Self { rt, lambda, iam })
    }
}

impl LambdaClient for AwsLambdaClient {
    fn check_connectivity(&self) -> Result<(), LambdaError> {
        self.rt.block_on(async {
            self.lambda
                .list_functions()
                .max_items(1)
                .send()
                .await
                .map_err(|e| {
                    LambdaError::Aws(format!("connectivity check: {}", format_sdk_error(&e)))
                })?;
            Ok(())
        })
    }

    fn create_role(&self, role_name: &str) -> Result<String, LambdaError> {
        self.rt.block_on(async {
            match self
                .iam
                .create_role()
                .role_name(role_name)
                .assume_role_policy_document(LAMBDA_TRUST_POLICY)
                .send()
                .await
            {
                Ok(output) => {
                    let arn = output
                        .role()
                        .map(|r| r.arn().to_string())
                        .unwrap_or_default();
                    tracing::info!(role = role_name, arn = arn.as_str(), "Created IAM role");
                    Ok(arn)
                }
                Err(sdk_err) => {
                    // Idempotent: if the role already exists, fetch its ARN.
                    if is_entity_already_exists(&sdk_err) {
                        tracing::debug!(role = role_name, "IAM role already exists, fetching ARN");
                        let get = self
                            .iam
                            .get_role()
                            .role_name(role_name)
                            .send()
                            .await
                            .map_err(|e| {
                                LambdaError::Aws(format!(
                                    "create_role (get existing): {}",
                                    format_sdk_error(&e)
                                ))
                            })?;
                        let arn = get.role().map(|r| r.arn().to_string()).unwrap_or_default();
                        Ok(arn)
                    } else {
                        Err(LambdaError::Aws(format!(
                            "create_role: {}",
                            format_sdk_error(&sdk_err)
                        )))
                    }
                }
            }
        })
    }

    fn delete_role(&self, role_name: &str) {
        let result = self
            .rt
            .block_on(async { self.iam.delete_role().role_name(role_name).send().await });
        match result {
            Ok(_) => tracing::info!(role = role_name, "Deleted IAM role"),
            Err(e) => tracing::warn!(
                role = role_name,
                error = %format_sdk_error(&e),
                "Failed to delete IAM role"
            ),
        }
    }

    fn create_function(
        &self,
        function_name: &str,
        ecr_image_uri: &str,
        role_arn: &str,
        memory_mb: i32,
        timeout_secs: i32,
        env_vars: &[(&str, &str)],
    ) -> Result<String, LambdaError> {
        self.rt.block_on(async {
            let mut env_map = std::collections::HashMap::new();
            for (k, v) in env_vars {
                env_map.insert(k.to_string(), v.to_string());
            }
            let environment = aws_sdk_lambda::types::Environment::builder()
                .set_variables(Some(env_map))
                .build();

            let code = aws_sdk_lambda::types::FunctionCode::builder()
                .image_uri(ecr_image_uri)
                .build();

            match self
                .lambda
                .create_function()
                .function_name(function_name)
                .role(role_arn)
                .code(code)
                .package_type(aws_sdk_lambda::types::PackageType::Image)
                .memory_size(memory_mb)
                .timeout(timeout_secs)
                .environment(environment)
                .send()
                .await
            {
                Ok(output) => {
                    let arn = output.function_arn().unwrap_or_default().to_string();
                    tracing::info!(
                        function = function_name,
                        arn = arn.as_str(),
                        "Created Lambda function"
                    );
                    Ok(arn)
                }
                Err(sdk_err) => {
                    // Idempotent: if the function already exists, fetch its ARN.
                    if is_resource_conflict(&sdk_err) {
                        tracing::debug!(
                            function = function_name,
                            "Lambda function already exists, fetching ARN"
                        );
                        let info = self.get_function_inner(function_name).await?;
                        Ok(info.map(|f| f.function_arn).unwrap_or_default())
                    } else {
                        Err(LambdaError::Aws(format!(
                            "create_function: {}",
                            format_sdk_error(&sdk_err)
                        )))
                    }
                }
            }
        })
    }

    fn get_function(&self, function_name: &str) -> Result<Option<FunctionInfo>, LambdaError> {
        self.rt
            .block_on(async { self.get_function_inner(function_name).await })
    }

    fn delete_function(&self, function_name: &str) {
        let result = self.rt.block_on(async {
            self.lambda
                .delete_function()
                .function_name(function_name)
                .send()
                .await
        });
        match result {
            Ok(_) => tracing::info!(function = function_name, "Deleted Lambda function"),
            Err(e) => tracing::warn!(
                function = function_name,
                error = %format_sdk_error(&e),
                "Failed to delete Lambda function"
            ),
        }
    }
}

impl AwsLambdaClient {
    /// Shared async helper for get_function (used by both get_function and create_function).
    async fn get_function_inner(
        &self,
        function_name: &str,
    ) -> Result<Option<FunctionInfo>, LambdaError> {
        match self
            .lambda
            .get_function()
            .function_name(function_name)
            .send()
            .await
        {
            Ok(output) => {
                let arn = output
                    .configuration()
                    .and_then(|c| c.function_arn())
                    .unwrap_or_default()
                    .to_string();
                Ok(Some(FunctionInfo { function_arn: arn }))
            }
            Err(sdk_err) => {
                if is_resource_not_found(&sdk_err) {
                    Ok(None)
                } else {
                    Err(LambdaError::Aws(format!(
                        "get_function: {}",
                        format_sdk_error(&sdk_err)
                    )))
                }
            }
        }
    }
}

// ── SDK error helpers ───────────────────────────────────────────────

/// Extract a human-readable error message from an AWS SDK error.
///
/// The SDK's `Display` impl just says "service error" — useless.
/// This walks the error source chain to find the actual error code and message.
fn format_sdk_error<E: std::error::Error>(err: &E) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(e) = source {
        parts.push(e.to_string());
        source = e.source();
    }
    parts.join(": ")
}

fn is_entity_already_exists<E: std::fmt::Debug>(err: &aws_sdk_iam::error::SdkError<E>) -> bool {
    format!("{:?}", err).contains("EntityAlreadyExists")
}

fn is_resource_conflict<E: std::fmt::Debug>(err: &aws_sdk_lambda::error::SdkError<E>) -> bool {
    format!("{:?}", err).contains("ResourceConflictException")
}

fn is_resource_not_found<E: std::fmt::Debug>(err: &aws_sdk_lambda::error::SdkError<E>) -> bool {
    format!("{:?}", err).contains("ResourceNotFoundException")
}
