//! LambdaRuntime — manages AWS Lambda functions for Lambda-typed agents.
//!
//! Reconciliation loop mirrors `ContainerRuntime` (pool.rs):
//! query registry → remove orphan functions → deploy missing agents.

use std::collections::HashMap;
use std::sync::Arc;

use vlinder_core::domain::{Agent, Registry, ResourceId, Runtime, RuntimeType};

use crate::config::LambdaRuntimeConfig;
use crate::lambda_client::{AwsLambdaClient, LambdaClient, LambdaError};

/// A deployed Lambda function: IAM role + Lambda function.
#[allow(dead_code)]
struct DeployedFunction {
    role_arn: String,
    function_arn: String,
}

/// Orchestrates AWS Lambda functions for Lambda-typed agents.
///
/// Maps agent names to deployed functions. Each agent gets:
/// - An IAM role `vlinder-agent-{name}` with zero permissions
/// - A Lambda function `vlinder-{name}` running the agent's ECR image
pub struct LambdaRuntime {
    id: ResourceId,
    registry: Arc<dyn Registry>,
    functions: HashMap<String, DeployedFunction>,
    config: LambdaRuntimeConfig,
    client: Box<dyn LambdaClient>,
}

impl LambdaRuntime {
    /// Create a new Lambda runtime connected to the given registry.
    ///
    /// Returns `Err` if the AWS client cannot be initialized (bad region, no creds).
    pub fn new(
        config: &LambdaRuntimeConfig,
        registry: Arc<dyn Registry>,
    ) -> Result<Self, LambdaError> {
        let client = AwsLambdaClient::new(&config.region)?;
        Self::with_client(config, registry, Box::new(client))
    }

    /// Create a runtime with an injected client (for testing).
    fn with_client(
        config: &LambdaRuntimeConfig,
        registry: Arc<dyn Registry>,
        client: Box<dyn LambdaClient>,
    ) -> Result<Self, LambdaError> {
        let registry_id = ResourceId::new(&config.registry_addr);
        let id = ResourceId::new(format!(
            "{}/runtimes/{}",
            registry_id.as_str(),
            RuntimeType::Lambda.as_str()
        ));

        Ok(Self {
            id,
            registry,
            functions: HashMap::new(),
            config: config.clone(),
            client,
        })
    }

    /// Reconcile deployed functions with registry state.
    ///
    /// 1. Query registry for Lambda agents.
    /// 2. Remove orphan functions (deployed but no longer in registry).
    /// 3. Deploy missing agents (in registry but not deployed).
    ///
    /// Returns true if the function count changed.
    fn ensure_functions(&mut self) -> bool {
        let agents = self.registry.get_agents_by_runtime(RuntimeType::Lambda);
        let desired: HashMap<String, &Agent> = agents.iter().map(|a| (a.name.clone(), a)).collect();

        let before = self.functions.len();

        // Remove orphans: deployed but not in registry.
        let orphans: Vec<String> = self
            .functions
            .keys()
            .filter(|name| !desired.contains_key(*name))
            .cloned()
            .collect();

        for name in orphans {
            tracing::info!(agent = name.as_str(), "Removing orphan Lambda function");
            self.undeploy(&name);
        }

        // Deploy missing: in registry but not deployed.
        for (name, agent) in &desired {
            if !self.functions.contains_key(name) {
                tracing::info!(agent = name.as_str(), "Deploying Lambda function");
                if let Err(e) = self.deploy(name, agent) {
                    tracing::error!(
                        agent = name.as_str(),
                        error = %e,
                        "Failed to deploy Lambda function"
                    );
                }
            }
        }

        self.functions.len() != before
    }

    /// Deploy a single agent as a Lambda function.
    fn deploy(&mut self, name: &str, agent: &Agent) -> Result<(), LambdaError> {
        let role_name = format!("vlinder-agent-{}", name);
        let function_name = format!("vlinder-{}", name);

        let role_arn = self.client.create_role(&role_name)?;

        let env_vars: Vec<(&str, &str)> = vec![("VLINDER_AGENT", &agent.name)];

        let function_arn = self.client.create_function(
            &function_name,
            &agent.executable,
            &role_arn,
            self.config.memory_mb,
            self.config.timeout_secs,
            &env_vars,
        )?;

        self.functions.insert(
            name.to_string(),
            DeployedFunction {
                role_arn,
                function_arn,
            },
        );

        tracing::info!(
            agent = name,
            function = function_name.as_str(),
            "Lambda function deployed"
        );

        Ok(())
    }

    /// Tear down a deployed function: delete Lambda first, then IAM role.
    fn undeploy(&mut self, name: &str) {
        let function_name = format!("vlinder-{}", name);
        let role_name = format!("vlinder-agent-{}", name);

        self.client.delete_function(&function_name);
        self.client.delete_role(&role_name);

        self.functions.remove(name);
    }
}

impl Runtime for LambdaRuntime {
    fn id(&self) -> &ResourceId {
        &self.id
    }

    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Lambda
    }

    fn tick(&mut self) -> bool {
        self.ensure_functions()
    }

    fn shutdown(&mut self) {
        let names: Vec<String> = self.functions.keys().cloned().collect();
        for name in names {
            tracing::info!(agent = name.as_str(), "Shutting down Lambda function");
            self.undeploy(&name);
        }
    }
}

impl Drop for LambdaRuntime {
    fn drop(&mut self) {
        if !self.functions.is_empty() {
            self.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashSet;
    use vlinder_core::domain::InMemorySecretStore;

    // ── Mock client ─────────────────────────────────────────────────

    struct MockLambdaClient {
        roles: RefCell<HashSet<String>>,
        functions: RefCell<HashSet<String>>,
    }

    impl MockLambdaClient {
        fn new() -> Self {
            Self {
                roles: RefCell::new(HashSet::new()),
                functions: RefCell::new(HashSet::new()),
            }
        }
    }

    impl LambdaClient for MockLambdaClient {
        fn check_connectivity(&self) -> Result<(), LambdaError> {
            Ok(())
        }

        fn create_role(&self, role_name: &str) -> Result<String, LambdaError> {
            self.roles.borrow_mut().insert(role_name.to_string());
            Ok(format!("arn:aws:iam::123456789012:role/{}", role_name))
        }

        fn delete_role(&self, role_name: &str) {
            self.roles.borrow_mut().remove(role_name);
        }

        fn create_function(
            &self,
            function_name: &str,
            _ecr_image_uri: &str,
            _role_arn: &str,
            _memory_mb: i32,
            _timeout_secs: i32,
            _env_vars: &[(&str, &str)],
        ) -> Result<String, LambdaError> {
            self.functions
                .borrow_mut()
                .insert(function_name.to_string());
            Ok(format!(
                "arn:aws:lambda:us-east-1:123456789012:function:{}",
                function_name
            ))
        }

        fn get_function(
            &self,
            function_name: &str,
        ) -> Result<Option<crate::lambda_client::FunctionInfo>, LambdaError> {
            if self.functions.borrow().contains(function_name) {
                Ok(Some(crate::lambda_client::FunctionInfo {
                    function_arn: format!(
                        "arn:aws:lambda:us-east-1:123456789012:function:{}",
                        function_name
                    ),
                }))
            } else {
                Ok(None)
            }
        }

        fn delete_function(&self, function_name: &str) {
            self.functions.borrow_mut().remove(function_name);
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────

    fn test_config() -> LambdaRuntimeConfig {
        LambdaRuntimeConfig {
            registry_addr: "http://127.0.0.1:9090".to_string(),
            region: "us-east-1".to_string(),
            memory_mb: 512,
            timeout_secs: 300,
        }
    }

    fn test_registry() -> Arc<dyn Registry> {
        use vlinder_core::domain::InMemoryRegistry;
        let secret_store = Arc::new(InMemorySecretStore::new());
        let registry = Arc::new(InMemoryRegistry::new(secret_store));
        registry.register_runtime(RuntimeType::Lambda);
        registry
    }

    fn make_lambda_agent(name: &str) -> Agent {
        Agent::from_toml(&format!(
            r#"
            name = "{name}"
            description = "Test Lambda agent"
            runtime = "lambda"
            executable = "123456789012.dkr.ecr.us-east-1.amazonaws.com/{name}:latest"

            [requirements]
            "#
        ))
        .unwrap()
    }

    fn make_runtime(registry: Arc<dyn Registry>) -> LambdaRuntime {
        let config = test_config();
        LambdaRuntime::with_client(&config, registry, Box::new(MockLambdaClient::new())).unwrap()
    }

    // ── Tests ───────────────────────────────────────────────────────

    #[test]
    fn runtime_id_format() {
        let registry = test_registry();
        let runtime = make_runtime(registry);

        assert_eq!(
            runtime.id().as_str(),
            "http://127.0.0.1:9090/runtimes/lambda"
        );
        assert_eq!(runtime.runtime_type(), RuntimeType::Lambda);
    }

    #[test]
    fn tick_returns_false_when_no_agents() {
        let registry = test_registry();
        let mut runtime = make_runtime(registry);

        assert!(!runtime.tick());
        assert!(runtime.functions.is_empty());
    }

    #[test]
    fn tick_deploys_new_agent() {
        let registry = test_registry();
        let agent = make_lambda_agent("echo");
        registry.register_agent(agent).unwrap();

        let mut runtime = make_runtime(registry);

        // First tick: deploys the agent → count changed.
        assert!(runtime.tick());
        assert_eq!(runtime.functions.len(), 1);
        assert!(runtime.functions.contains_key("echo"));

        // Second tick: no change.
        assert!(!runtime.tick());
    }

    #[test]
    fn tick_removes_orphan() {
        let registry = test_registry();
        let agent = make_lambda_agent("echo");
        registry.register_agent(agent).unwrap();

        let mut runtime = make_runtime(registry.clone());
        runtime.tick(); // deploys
        assert_eq!(runtime.functions.len(), 1);

        // Remove from registry → next tick undeploys.
        registry.delete_agent("echo").unwrap();
        assert!(runtime.tick());
        assert!(runtime.functions.is_empty());
    }

    #[test]
    fn shutdown_undeploys_all() {
        let registry = test_registry();
        registry.register_agent(make_lambda_agent("alpha")).unwrap();
        registry.register_agent(make_lambda_agent("beta")).unwrap();

        let mut runtime = make_runtime(registry);
        runtime.tick();
        assert_eq!(runtime.functions.len(), 2);

        runtime.shutdown();
        assert!(runtime.functions.is_empty());
    }

    #[test]
    fn deploy_creates_role_and_function_with_correct_names() {
        let registry = test_registry();
        let agent = make_lambda_agent("echo");
        registry.register_agent(agent).unwrap();

        let mock = MockLambdaClient::new();
        let config = test_config();
        let mut runtime = LambdaRuntime::with_client(&config, registry, Box::new(mock)).unwrap();

        runtime.tick();

        let deployed = &runtime.functions["echo"];
        assert!(deployed.role_arn.contains("vlinder-agent-echo"));
        assert!(deployed.function_arn.contains("vlinder-echo"));
    }
}
