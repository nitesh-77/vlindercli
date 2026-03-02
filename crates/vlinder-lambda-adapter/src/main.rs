//! vlinder-lambda-adapter — Lambda extension that gives agents provider services.
//!
//! Replaces `aws-lambda-web-adapter` inside Lambda container images. Speaks the
//! Lambda Runtime API on one side and runs a full ProviderServer on the other,
//! giving Lambda agents access to inference, KV, vector storage, and delegation.
//!
//! Lifecycle:
//! 1. Read config from env
//! 2. Connect to NATS + registry + state
//! 3. Wait for agent to be ready on localhost
//! 4. Enter Lambda Runtime API loop:
//!    a. GET /runtime/invocation/next (blocks until Lambda dispatches)
//!    b. Deserialize InvokeMessage from body
//!    c. Start ProviderServer, POST payload to agent
//!    d. Build complete message with diagnostics and state
//!    e. Send complete to NATS
//!    f. POST response back to Lambda Runtime API

mod config;

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vlinder_core::domain::{
    InvokeMessage, MessageQueue, Registry, RuntimeDiagnostics, RuntimeInfo,
};

use vlinder_provider_server::factory;
use vlinder_provider_server::provider_server::{build_hosts, ProviderServer};

use config::AdapterConfig;

fn main() {
    let filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "warn,vlinder_lambda_adapter=info".to_string());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config = match AdapterConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to parse adapter config from env");
            std::process::exit(1);
        }
    };

    tracing::info!(
        event = "adapter.config",
        agent = %config.agent,
        runtime_api = %config.runtime_api,
        nats_url = %config.nats_url,
        registry_url = %config.registry_url,
        state_url = %config.state_url,
        agent_port = config.agent_port,
        "Lambda adapter configuration loaded"
    );

    let queue = match factory::connect_queue(
        &config.nats_url,
        &config.state_url,
        config.secret_url.as_deref(),
    ) {
        Ok(q) => q,
        Err(e) => {
            tracing::error!(error = %e, "Failed to connect to NATS");
            std::process::exit(1);
        }
    };

    let registry = match factory::connect_registry(&config.registry_url) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "Failed to connect to registry");
            std::process::exit(1);
        }
    };

    let http = ureq::Agent::new();

    if let Err(e) = wait_for_agent(&http, config.agent_port) {
        tracing::error!(error = %e, "Agent did not become ready");
        std::process::exit(1);
    }

    tracing::info!(event = "adapter.started", agent = %config.agent, "Entering Runtime API loop");

    if let Err(e) = runtime_api_loop(&config, &http, &queue, &registry) {
        tracing::error!(error = %e, "Runtime API loop exited with error");
        std::process::exit(1);
    }
}

/// Block until the agent's health endpoint responds (up to 60s).
fn wait_for_agent(http: &ureq::Agent, port: u16) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{}/health", port);
    let deadline = Instant::now() + Duration::from_secs(60);

    tracing::info!(
        event = "adapter.waiting",
        port = port,
        "Waiting for agent to become ready"
    );

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "agent did not become ready within 60s (port {})",
                port
            ));
        }
        if http.get(&url).call().is_ok() {
            tracing::info!(event = "adapter.agent_ready", "Agent is ready");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Main loop: poll Lambda Runtime API, dispatch to agent, respond.
fn runtime_api_loop(
    config: &AdapterConfig,
    http: &ureq::Agent,
    queue: &Arc<dyn MessageQueue + Send + Sync>,
    registry: &Arc<dyn Registry>,
) -> Result<(), String> {
    let next_url = format!(
        "http://{}/2018-06-01/runtime/invocation/next",
        config.runtime_api,
    );

    loop {
        // Block until Lambda dispatches an invocation.
        let response = http
            .get(&next_url)
            .call()
            .map_err(|e| format!("GET invocation/next failed: {}", e))?;

        let request_id = response
            .header("Lambda-Runtime-Aws-Request-Id")
            .unwrap_or("unknown")
            .to_string();

        let mut body = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| format!("failed to read invocation body: {}", e))?;

        tracing::info!(
            event = "adapter.invocation",
            request_id = %request_id,
            body_bytes = body.len(),
            "Received Lambda invocation"
        );

        match handle_invocation(config, http, queue, registry, &request_id, &body) {
            Ok(output) => {
                let response_url = format!(
                    "http://{}/2018-06-01/runtime/invocation/{}/response",
                    config.runtime_api, request_id,
                );
                http.post(&response_url)
                    .send_bytes(&output)
                    .map_err(|e| format!("POST invocation response failed: {}", e))?;
            }
            Err(e) => {
                tracing::error!(
                    event = "adapter.invocation_error",
                    request_id = %request_id,
                    error = %e,
                    "Invocation failed"
                );
                let error_url = format!(
                    "http://{}/2018-06-01/runtime/invocation/{}/error",
                    config.runtime_api, request_id,
                );
                let error_body = serde_json::json!({
                    "errorMessage": e,
                    "errorType": "AdapterError",
                });
                let _ = http
                    .post(&error_url)
                    .send_bytes(error_body.to_string().as_bytes());
            }
        }
    }
}

/// Handle a single Lambda invocation.
///
/// The invocation body is a JSON-serialized InvokeMessage (sent by the daemon).
/// We deserialize it, start a ProviderServer, POST the payload to the agent,
/// build diagnostics, send complete to NATS, and return the agent's output.
fn handle_invocation(
    config: &AdapterConfig,
    http: &ureq::Agent,
    queue: &Arc<dyn MessageQueue + Send + Sync>,
    registry: &Arc<dyn Registry>,
    request_id: &str,
    body: &[u8],
) -> Result<Vec<u8>, String> {
    let invoke: InvokeMessage = serde_json::from_slice(body)
        .map_err(|e| format!("failed to deserialize InvokeMessage: {}", e))?;

    let started_at = Instant::now();

    // Look up agent for provider host table and initial state.
    let agent = registry
        .get_agent_by_name(invoke.agent_id.as_str())
        .ok_or_else(|| format!("agent '{}' not found in registry", invoke.agent_id))?;
    let hosts = build_hosts(&agent);
    let initial_state = if agent.object_storage.is_some() {
        Some(invoke.state.clone().unwrap_or_default())
    } else {
        None
    };

    // Spawn provider server — drops when this function returns.
    let provider_server = ProviderServer::start(
        &invoke,
        hosts,
        queue.clone(),
        registry.clone(),
        initial_state,
        80,
    );

    // POST payload to agent on localhost.
    let agent_url = format!("http://127.0.0.1:{}/invoke", config.agent_port);
    let agent_response = http
        .post(&agent_url)
        .send_bytes(&invoke.payload)
        .map_err(|e| format!("POST to agent failed: {}", e))?;

    let mut output = Vec::new();
    agent_response
        .into_reader()
        .read_to_end(&mut output)
        .map_err(|e| format!("failed to read agent response: {}", e))?;

    let final_state = provider_server.final_state();
    let duration_ms = started_at.elapsed().as_millis() as u64;

    // Determine region from env (set by Lambda service).
    let region = std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|_| "unknown".to_string());

    let diagnostics = RuntimeDiagnostics {
        stderr: Vec::new(),
        runtime: RuntimeInfo::Lambda {
            function_name: config.agent.clone(),
            region,
        },
        duration_ms,
    };

    let complete = invoke.create_reply_with_diagnostics(output.clone(), final_state, diagnostics);
    queue
        .send_complete(complete)
        .map_err(|e| format!("failed to send complete to NATS: {}", e))?;

    tracing::info!(
        event = "adapter.invocation_complete",
        request_id = %request_id,
        duration_ms = duration_ms,
        output_bytes = output.len(),
        "Invocation complete"
    );

    Ok(output)
}
