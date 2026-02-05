//! WasmRuntime - executes WASM agents in response to queue messages.
//!
//! The runtime:
//! - Registers agents to serve
//! - Polls their input queues
//! - Executes WASM on message arrival
//! - Sends responses to reply queues
//!
//! Host functions provided to WASM guests:
//! - send(payload) -> response (synchronous request/response)
//!
//! The runtime handles all routing concerns:
//! - Extracts `op` from payload to determine target queue
//! - Injects `agent_id` for storage isolation
//! - Manages reply queue creation and response waiting

use std::sync::Arc;
use std::thread::{self, JoinHandle};

use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, Wasm};

use crate::domain::{Registry, ResourceId, Runtime, RuntimeType};
use crate::queue::{
    ExpectsReply, InvokeMessage, MessageQueue, RequestMessage, Sequence, SubmissionId,
};

/// Tracks a WASM execution running in a background thread.
struct RunningTask {
    handle: JoinHandle<Vec<u8>>,
    /// The original invoke message - used to construct CompleteMessage reply
    invoke: InvokeMessage,
}

pub struct WasmRuntime {
    id: ResourceId,
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    running: Option<RunningTask>,
}

impl WasmRuntime {
    pub fn new(
        registry_id: &ResourceId,
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
    ) -> Self {
        let id = ResourceId::new(format!(
            "{}/runtimes/{}",
            registry_id.as_str(),
            RuntimeType::Wasm.as_str()
        ));
        Self {
            id,
            queue,
            registry,
            running: None,
        }
    }
}

impl Runtime for WasmRuntime {
    fn id(&self) -> &ResourceId {
        &self.id
    }

    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Wasm
    }

    fn tick(&mut self) -> bool {
        // First, check if a running task completed
        if let Some(task) = self.running.take() {
            if task.handle.is_finished() {
                // Task done - send CompleteMessage using typed reply (ADR 044)
                let output = task.handle.join().unwrap();
                let complete = task.invoke.create_reply(output);
                self.queue.send_complete(complete).unwrap();
                return true;
            } else {
                // Still running, put it back
                self.running = Some(task);
                return false; // Can't start new work while one is running
            }
        }

        // Collect WASM agents from registry
        let wasm_agents: Vec<_> = self.registry.get_agents()
            .into_iter()
            .filter(|agent| self.registry.select_runtime(agent) == Some(RuntimeType::Wasm))
            .collect();

        // Check for work using typed receive (ADR 044)
        for agent in wasm_agents {
            // Try to receive typed InvokeMessage
            if let Ok((invoke, ack)) = self.queue.receive_invoke(&agent.name) {
                let wasm_path = agent.id.path().unwrap_or(agent.id.as_str()).to_string();
                let agent_id = agent.id.clone();
                let queue = Arc::clone(&self.queue);
                let payload = invoke.payload.clone();
                let submission = invoke.submission.clone();

                // ACK the message now - we've taken ownership of processing
                let _ = ack();

                // TECH DEBT: We pass registry to the thread to look up agent config for routing.
                // This is wasteful - we already have the Agent here. Cleaner approach:
                // extract (kv_backend, vec_backend) from agent before spawn, pass a closure
                // to make_send_function. Deferring until we build another runtime (e.g., Docker)
                // to clarify the contracts between runtime and host functions.
                let registry_for_thread = Arc::clone(&self.registry);

                // Spawn WASM execution in background thread
                let handle = thread::spawn(move || {
                    let functions = [
                        make_send_function(Arc::clone(&queue), agent_id, registry_for_thread, submission),
                        make_get_prompts_function(),
                    ];

                    let wasm = Wasm::file(&wasm_path);
                    let manifest =
                        Manifest::new([wasm]).with_allowed_hosts(["*".to_string()].into_iter());
                    let mut plugin = Plugin::new(&manifest, functions, true).unwrap();

                    plugin.call::<_, Vec<u8>>("process", &payload).unwrap()
                });

                self.running = Some(RunningTask {
                    handle,
                    invoke,
                });

                return true; // Started new work
            }
        }

        false // No work to do
    }
}

// ============================================================================
// Host Function Builders
// ============================================================================

fn write_output(plugin: &mut CurrentPlugin, outputs: &mut [Val], response: &[u8]) -> Result<(), extism::Error> {
    let handle = plugin.memory_new(response)?;
    outputs[0] = Val::I64(handle.offset() as i64);
    Ok(())
}

/// Data passed to the send host function
struct SendFunctionData {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    agent_id: ResourceId,
    registry: Arc<dyn Registry>,
    /// Submission context for typed messages (ADR 044)
    submission: SubmissionId,
    /// Sequence counter - incremented per service call
    sequence: Arc<std::sync::Mutex<Sequence>>,
}

/// Host function: send(payload) -> response
///
/// Synchronous request/response. The runtime:
/// - Extracts `op` from payload to determine target queue
/// - Injects `agent_id` for storage isolation
/// - Builds typed RequestMessage with submission context (ADR 044)
/// - Returns the response payload directly
fn make_send_function(
    queue: Arc<dyn MessageQueue + Send + Sync>,
    agent_id: ResourceId,
    registry: Arc<dyn Registry>,
    submission: SubmissionId,
) -> Function {
    let sequence = Arc::new(std::sync::Mutex::new(Sequence::first()));
    let data = SendFunctionData { queue, agent_id, registry, submission, sequence };

    Function::new(
        "send",
        [extism::PTR], // payload (JSON with op field)
        [extism::PTR], // response payload
        UserData::new(data),
        |plugin, inputs, outputs, user_data| {
            let payload: Vec<u8> = plugin.memory_get_val(&inputs[0])?;

            let data = user_data.get().unwrap();
            let data = data.lock().unwrap();

            // Parse payload to extract op and inject agent_id
            let mut json: serde_json::Value = serde_json::from_slice(&payload)
                .map_err(|e| extism::Error::msg(format!("invalid JSON payload: {}", e)))?;

            // Extract op to determine queue
            let op = json.get("op")
                .and_then(|v| v.as_str())
                .ok_or_else(|| extism::Error::msg("payload missing 'op' field"))?
                .to_string();

            tracing::debug!(op = %op, "WASM agent calling service");

            // Inject agent_id for storage operations
            if let Some(obj) = json.as_object_mut() {
                obj.insert("agent_id".to_string(), serde_json::Value::String(data.agent_id.as_str().to_string()));
            }

            // Resolve service and backend from operation
            let (service, backend, operation) = resolve_service_info(&data.registry, data.agent_id.as_str(), &op)?;

            // Get next sequence number
            let seq = {
                let mut seq_lock = data.sequence.lock().unwrap();
                let current = *seq_lock;
                *seq_lock = seq_lock.next();
                current
            };

            // Build typed RequestMessage (ADR 044)
            let enriched_payload = serde_json::to_vec(&json)
                .map_err(|e| extism::Error::msg(format!("serialize error: {}", e)))?;

            let request = RequestMessage::new(
                data.submission.clone(),
                data.agent_id.clone(),
                service.clone(),
                backend.clone(),
                operation.clone(),
                seq,
                enriched_payload,
            );

            tracing::debug!(
                submission = %data.submission,
                service = %service,
                backend = %backend,
                operation = %operation,
                sequence = %seq,
                "Sending typed RequestMessage"
            );

            data.queue.send_request(request)
                .map_err(|e| extism::Error::msg(format!("send error: {}", e)))?;

            // Wait for typed ResponseMessage (ADR 044)
            let reply_pattern = format!("{}.res.{}.{}", data.submission, service, backend);
            loop {
                if let Ok((response, ack)) = data.queue.receive_response(&reply_pattern) {
                    let payload = response.payload.clone();
                    let _ = ack();
                    return write_output(plugin, outputs, &payload);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        },
    )
}

/// Resolve service, backend, and operation from an op string.
///
/// Returns (service, backend, operation) tuple for building typed RequestMessage.
/// Maps operation names to backend-specific info based on agent configuration:
/// - `kv-*` operations → look up agent's object_storage URI scheme
/// - `vector-*` operations → look up agent's vector_storage URI scheme
/// - `infer`/`embed` → hardcoded to "ollama" (only supported backend for now)
fn resolve_service_info(
    registry: &Arc<dyn Registry>,
    agent_id: &str,
    op: &str,
) -> Result<(String, String, String), extism::Error> {
    // Look up agent from registry
    let resource_id = ResourceId::new(agent_id);
    let agent = registry.get_agent(&resource_id)
        .ok_or_else(|| extism::Error::msg(format!("agent not found: {}", agent_id)))?;

    // Determine service, backend, and operation based on op string
    let (service, backend, operation) = match op {
        // KV storage operations
        "kv-get" | "kv-put" | "kv-list" | "kv-delete" => {
            let backend = agent.object_storage.as_ref()
                .and_then(|uri| uri.as_str().split("://").next())
                .unwrap_or("memory")
                .to_string();
            let operation = op.strip_prefix("kv-").unwrap_or(op).to_string();
            ("kv".to_string(), backend, operation)
        }
        // Vector storage operations
        "vector-store" | "vector-search" | "vector-delete" => {
            let backend = agent.vector_storage.as_ref()
                .and_then(|uri| uri.as_str().split("://").next())
                .map(|s| if s == "sqlite" { "sqlite-vec" } else { s })
                .unwrap_or("memory")
                .to_string();
            let operation = op.strip_prefix("vector-").unwrap_or(op).to_string();
            ("vec".to_string(), backend, operation)
        }
        // Inference (currently only ollama supported)
        "infer" => ("infer".to_string(), "ollama".to_string(), String::new()),
        // Embedding (currently only ollama supported)
        "embed" => ("embed".to_string(), "ollama".to_string(), String::new()),
        // Unknown operation - treat as agent-to-agent message
        _ => ("agent".to_string(), op.to_string(), String::new()),
    };

    Ok((service, backend, operation))
}

/// Resolve the backend-qualified queue name for a service operation.
///
/// Maps operation names to backend-specific queues based on agent configuration:
/// - `kv-*` operations → look up agent's object_storage URI scheme
/// - `vector-*` operations → look up agent's vector_storage URI scheme
/// - `infer`/`embed` → hardcoded to "ollama" (only supported backend for now)
#[allow(dead_code)] // Will be removed after full migration to typed messages
fn resolve_service_queue(
    queue: &Arc<dyn MessageQueue + Send + Sync>,
    registry: &Arc<dyn Registry>,
    agent_id: &str,
    op: &str,
) -> Result<String, extism::Error> {
    // Look up agent from registry
    let resource_id = ResourceId::new(agent_id);
    let agent = registry.get_agent(&resource_id)
        .ok_or_else(|| extism::Error::msg(format!("agent not found: {}", agent_id)))?;

    // Determine service and backend based on operation
    let queue_name = match op {
        // KV storage operations
        "kv-get" | "kv-put" | "kv-list" | "kv-delete" => {
            let backend = agent.object_storage.as_ref()
                .and_then(|uri| uri.as_str().split("://").next())
                .unwrap_or("memory");
            let action = op.strip_prefix("kv-").unwrap_or(op);
            queue.service_queue("kv", backend, action)
        }
        // Vector storage operations
        "vector-store" | "vector-search" | "vector-delete" => {
            let backend = agent.vector_storage.as_ref()
                .and_then(|uri| uri.as_str().split("://").next())
                .map(|s| if s == "sqlite" { "sqlite-vec" } else { s })
                .unwrap_or("memory");
            let action = op.strip_prefix("vector-").unwrap_or(op);
            queue.service_queue("vec", backend, action)
        }
        // Inference (currently only ollama supported)
        "infer" => queue.service_queue("infer", "ollama", ""),
        // Embedding (currently only ollama supported)
        "embed" => queue.service_queue("embed", "ollama", ""),
        // Unknown operation - treat as agent-to-agent message
        _ => format!("vlinder.agent.{}", op),
    };

    Ok(queue_name)
}

/// Host function: get_prompts() -> JSON string
///
/// Returns prompt overrides from the agent's manifest.
/// Returns empty JSON object if no overrides defined.
fn make_get_prompts_function() -> Function {
    Function::new(
        "get_prompts",
        [],           // no inputs
        [extism::PTR], // JSON string
        UserData::new(()),
        |plugin, _inputs, outputs, _user_data| {
            // Return empty object - prompts default to compiled-in values
            write_output(plugin, outputs, b"{}")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::InMemoryRegistry;
    use crate::queue::InMemoryQueue;

    fn test_registry_id() -> ResourceId {
        ResourceId::new("http://test:9000")
    }

    fn test_registry() -> Arc<dyn Registry> {
        Arc::new(InMemoryRegistry::new())
    }

    #[test]
    fn runtime_id_format() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let runtime = WasmRuntime::new(&test_registry_id(), queue, test_registry());

        // Format: <registry_id>/runtimes/<runtime_type>
        assert_eq!(runtime.id().as_str(), "http://test:9000/runtimes/wasm");
        assert_eq!(runtime.runtime_type(), RuntimeType::Wasm);
    }
}
