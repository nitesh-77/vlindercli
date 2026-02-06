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

use crate::domain::{ObjectStorageType, Registry, ResourceId, Runtime, RuntimeType, VectorStorageType};
use crate::queue::{
    ExpectsReply, InvokeMessage, MessageQueue, RequestMessage, Sequence,
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
                let queue = Arc::clone(&self.queue);
                let payload = invoke.payload.clone();

                // ACK the message now - we've taken ownership of processing
                let _ = ack();

                // Extract storage backends from agent config before spawning
                let kv_backend = agent.object_storage.as_ref()
                    .and_then(|uri| ObjectStorageType::from_scheme(uri.scheme()));
                let vec_backend = agent.vector_storage.as_ref()
                    .and_then(|uri| VectorStorageType::from_scheme(uri.scheme()));

                // Clone invoke for the send function; original stays for RunningTask
                let invoke_for_send = invoke.clone();

                // Spawn WASM execution in background thread
                let handle = thread::spawn(move || {
                    let functions = [
                        make_send_function(Arc::clone(&queue), invoke_for_send, kv_backend, vec_backend),
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
    /// The invoke that triggered this execution - carries submission + agent_id
    invoke: InvokeMessage,
    /// Resolved backends from agent config (None if agent didn't declare storage)
    kv_backend: Option<ObjectStorageType>,
    vec_backend: Option<VectorStorageType>,
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
    invoke: InvokeMessage,
    kv_backend: Option<ObjectStorageType>,
    vec_backend: Option<VectorStorageType>,
) -> Function {
    let sequence = Arc::new(std::sync::Mutex::new(Sequence::first()));
    let data: SendFunctionData = SendFunctionData { queue, invoke, kv_backend, vec_backend, sequence };

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
            
            // Inject agent_id for storage operations
            if let Some(obj) = json.as_object_mut() {
                obj.insert("agent_id".to_string(), serde_json::Value::String(data.invoke.agent_id.as_str().to_string()));
            }

            let (service, backend, operation) = resolve_route(&op, data.kv_backend, data.vec_backend)
                .map_err(|e| extism::Error::msg(e))?;

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
                data.invoke.submission.clone(),
                data.invoke.agent_id.clone(),
                service,
                backend,
                operation,
                seq,
                enriched_payload,
            );

            data.queue.send_request(request.clone())
                .map_err(|e| extism::Error::msg(format!("send error: {}", e)))?;

            // Wait for typed ResponseMessage (ADR 044)
            loop {
                if let Ok((response, ack)) = data.queue.receive_response(&request) {
                    let payload = response.payload.clone();
                    let _ = ack();
                    return write_output(plugin, outputs, &payload);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        },
    )
}

/// Map an op string to (service, backend, operation) for routing.
///
/// Pure routing table — no I/O, no registry lookups.
/// - `kv-*` → kv service with agent's configured object storage backend
/// - `vector-*` → vec service with agent's configured vector storage backend
/// - `infer`/`embed` → hardcoded to ollama (only supported backend)
/// - anything else → agent-to-agent message
///
/// Returns Err if the agent calls a storage op without declaring the backend.
fn resolve_route(
    op: &str,
    kv_backend: Option<ObjectStorageType>,
    vec_backend: Option<VectorStorageType>,
) -> Result<(String, String, String), String> {
    match op {
        "kv-get" | "kv-put" | "kv-list" | "kv-delete" => {
            let backend = kv_backend
                .ok_or_else(|| format!("agent called {} but has no object_storage configured", op))?;
            let operation = op.strip_prefix("kv-").unwrap_or(op).to_string();
            Ok(("kv".to_string(), backend.as_str().to_string(), operation))
        }
        "vector-store" | "vector-search" | "vector-delete" => {
            let backend = vec_backend
                .ok_or_else(|| format!("agent called {} but has no vector_storage configured", op))?;
            let operation = op.strip_prefix("vector-").unwrap_or(op).to_string();
            Ok(("vec".to_string(), backend.as_str().to_string(), operation))
        }
        "infer" => Ok(("infer".to_string(), "ollama".to_string(), "run".to_string())),
        "embed" => Ok(("embed".to_string(), "ollama".to_string(), "run".to_string())),
        _ => Ok(("agent".to_string(), op.to_string(), String::new())),
    }
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
    fn resolve_route_kv_operations() {
        let (svc, backend, op) = resolve_route("kv-get", Some(ObjectStorageType::Sqlite), None).unwrap();
        assert_eq!((svc.as_str(), backend.as_str(), op.as_str()), ("kv", "sqlite", "get"));

        let (svc, backend, op) = resolve_route("kv-put", Some(ObjectStorageType::InMemory), None).unwrap();
        assert_eq!((svc.as_str(), backend.as_str(), op.as_str()), ("kv", "memory", "put"));
    }

    #[test]
    fn resolve_route_vector_operations() {
        let (svc, backend, op) = resolve_route("vector-store", None, Some(VectorStorageType::SqliteVec)).unwrap();
        assert_eq!((svc.as_str(), backend.as_str(), op.as_str()), ("vec", "sqlite-vec", "store"));

        let (svc, backend, op) = resolve_route("vector-search", None, Some(VectorStorageType::InMemory)).unwrap();
        assert_eq!((svc.as_str(), backend.as_str(), op.as_str()), ("vec", "memory", "search"));
    }

    #[test]
    fn resolve_route_kv_without_backend_fails() {
        let err = resolve_route("kv-get", None, Some(VectorStorageType::InMemory)).unwrap_err();
        assert!(err.contains("no object_storage configured"));
    }

    #[test]
    fn resolve_route_vector_without_backend_fails() {
        let err = resolve_route("vector-store", Some(ObjectStorageType::Sqlite), None).unwrap_err();
        assert!(err.contains("no vector_storage configured"));
    }

    #[test]
    fn resolve_route_inference_ignores_backends() {
        let (svc, backend, op) = resolve_route("infer", None, None).unwrap();
        assert_eq!((svc.as_str(), backend.as_str(), op.as_str()), ("infer", "ollama", "run"));

        let (svc, backend, op) = resolve_route("embed", None, None).unwrap();
        assert_eq!((svc.as_str(), backend.as_str(), op.as_str()), ("embed", "ollama", "run"));
    }

    #[test]
    fn resolve_route_unknown_op_is_agent_message() {
        let (svc, backend, op) = resolve_route("summarize", None, None).unwrap();
        assert_eq!((svc.as_str(), backend.as_str(), op.as_str()), ("agent", "summarize", ""));
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
