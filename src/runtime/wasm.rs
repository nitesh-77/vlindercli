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
//! - Validates payload against SdkMessage schema (typed trust boundary)
//! - Carries `agent_id` on the message envelope (not in payload)
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

// ============================================================================
// SDK Message Schema
// ============================================================================

/// All valid operations a WASM agent can request through send().
///
/// This is the trust boundary — untrusted WASM payloads are validated here.
/// The `op` field determines the variant; remaining fields must match exactly.
/// Unknown ops or missing fields are rejected at deserialization time.
/// Fields are validated by serde but not read directly — the raw payload
/// bytes are forwarded to service workers for their own deserialization.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
#[allow(dead_code)]
enum SdkMessage {
    // -- Object storage --
    KvGet { path: String },
    KvPut { path: String, content: String },
    KvList { path: String },
    KvDelete { path: String },
    // -- Vector storage --
    VectorStore { key: String, vector: Vec<f32>, metadata: String },
    VectorSearch { vector: Vec<f32>, limit: u32 },
    VectorDelete { key: String },
    // -- Inference --
    Infer { model: String, prompt: String, #[serde(default = "default_max_tokens")] max_tokens: u32 },
    // -- Embedding --
    Embed { model: String, text: String },
}

fn default_max_tokens() -> u32 { 256 }

impl SdkMessage {
    /// Determine (service, backend, operation) for queue routing.
    ///
    /// Returns Err if the agent calls a storage op without declaring the backend.
    fn route(
        &self,
        kv_backend: Option<ObjectStorageType>,
        vec_backend: Option<VectorStorageType>,
    ) -> Result<(&'static str, String, &'static str), String> {
        match self {
            SdkMessage::KvGet { .. } => kv_route("get", kv_backend),
            SdkMessage::KvPut { .. } => kv_route("put", kv_backend),
            SdkMessage::KvList { .. } => kv_route("list", kv_backend),
            SdkMessage::KvDelete { .. } => kv_route("delete", kv_backend),
            SdkMessage::VectorStore { .. } => vec_route("store", vec_backend),
            SdkMessage::VectorSearch { .. } => vec_route("search", vec_backend),
            SdkMessage::VectorDelete { .. } => vec_route("delete", vec_backend),
            SdkMessage::Infer { .. } => Ok(("infer", "ollama".to_string(), "run")),
            SdkMessage::Embed { .. } => Ok(("embed", "ollama".to_string(), "run")),
        }
    }
}

fn kv_route(operation: &'static str, backend: Option<ObjectStorageType>) -> Result<(&'static str, String, &'static str), String> {
    let backend = backend.ok_or_else(|| format!("agent called kv-{} but has no object_storage configured", operation))?;
    Ok(("kv", backend.as_str().to_string(), operation))
}

fn vec_route(operation: &'static str, backend: Option<VectorStorageType>) -> Result<(&'static str, String, &'static str), String> {
    let backend = backend.ok_or_else(|| format!("agent called vector-{} but has no vector_storage configured", operation))?;
    Ok(("vec", backend.as_str().to_string(), operation))
}

/// Host function: send(payload) -> response
///
/// Synchronous request/response. The runtime:
/// - Validates payload against SdkMessage schema
/// - Builds typed RequestMessage with submission context (ADR 044)
/// - `agent_id` is carried on the envelope, not injected into payload
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
        [extism::PTR], // payload (SdkMessage JSON)
        [extism::PTR], // response payload
        UserData::new(data),
        |plugin, inputs, outputs, user_data| {
            let payload: Vec<u8> = plugin.memory_get_val(&inputs[0])?;

            let data = user_data.get().unwrap();
            let data = data.lock().unwrap();

            // Validate payload against SDK schema and determine route
            let msg: SdkMessage = serde_json::from_slice(&payload)
                .map_err(|e| extism::Error::msg(format!("invalid SDK message: {}", e)))?;

            let (service, backend, operation) = msg.route(data.kv_backend, data.vec_backend)
                .map_err(|e| extism::Error::msg(e))?;

            // Get next sequence number
            let seq = {
                let mut seq_lock = data.sequence.lock().unwrap();
                let current = *seq_lock;
                *seq_lock = seq_lock.next();
                current
            };

            // Build typed RequestMessage (ADR 044)
            // agent_id lives on the envelope, not in the payload
            let request = RequestMessage::new(
                data.invoke.submission.clone(),
                data.invoke.agent_id.clone(),
                service,
                backend,
                operation,
                seq,
                payload,
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

    /// Helper: parse JSON into SdkMessage and return its route.
    fn parse_and_route(
        json: &str,
        kv: Option<ObjectStorageType>,
        vec: Option<VectorStorageType>,
    ) -> Result<(&'static str, String, &'static str), String> {
        let msg: SdkMessage = serde_json::from_str(json)
            .map_err(|e| e.to_string())?;
        msg.route(kv, vec)
    }

    #[test]
    fn sdk_message_kv_operations() {
        let (svc, backend, op) = parse_and_route(
            r#"{"op": "kv-get", "path": "/data.txt"}"#,
            Some(ObjectStorageType::Sqlite), None,
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("kv", "sqlite", "get"));

        let (svc, backend, op) = parse_and_route(
            r#"{"op": "kv-put", "path": "/data.txt", "content": "aGVsbG8="}"#,
            Some(ObjectStorageType::InMemory), None,
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("kv", "memory", "put"));
    }

    #[test]
    fn sdk_message_vector_operations() {
        let (svc, backend, op) = parse_and_route(
            r#"{"op": "vector-store", "key": "doc1", "vector": [0.1], "metadata": "test"}"#,
            None, Some(VectorStorageType::SqliteVec),
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("vec", "sqlite-vec", "store"));

        let (svc, backend, op) = parse_and_route(
            r#"{"op": "vector-search", "vector": [0.1], "limit": 5}"#,
            None, Some(VectorStorageType::InMemory),
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("vec", "memory", "search"));
    }

    #[test]
    fn sdk_message_kv_without_backend_fails() {
        let err = parse_and_route(
            r#"{"op": "kv-get", "path": "/data.txt"}"#,
            None, None,
        ).unwrap_err();
        assert!(err.contains("no object_storage configured"));
    }

    #[test]
    fn sdk_message_vector_without_backend_fails() {
        let err = parse_and_route(
            r#"{"op": "vector-store", "key": "k", "vector": [0.1], "metadata": "m"}"#,
            None, None,
        ).unwrap_err();
        assert!(err.contains("no vector_storage configured"));
    }

    #[test]
    fn sdk_message_inference() {
        let (svc, backend, op) = parse_and_route(
            r#"{"op": "infer", "model": "phi3", "prompt": "hello"}"#,
            None, None,
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("infer", "ollama", "run"));

        let (svc, backend, op) = parse_and_route(
            r#"{"op": "embed", "model": "nomic", "text": "hello"}"#,
            None, None,
        ).unwrap();
        assert_eq!((svc, backend.as_str(), op), ("embed", "ollama", "run"));
    }

    #[test]
    fn sdk_message_unknown_op_fails() {
        let err = parse_and_route(
            r#"{"op": "summarize"}"#,
            None, None,
        ).unwrap_err();
        // serde rejects unknown variants
        assert!(err.contains("unknown variant"));
    }

    #[test]
    fn sdk_message_missing_fields_fails() {
        // kv-put without content
        let err = parse_and_route(
            r#"{"op": "kv-put", "path": "/data.txt"}"#,
            Some(ObjectStorageType::Sqlite), None,
        ).unwrap_err();
        assert!(err.contains("content"));
    }

    #[test]
    fn sdk_message_infer_defaults_max_tokens() {
        let msg: SdkMessage = serde_json::from_str(
            r#"{"op": "infer", "model": "phi3", "prompt": "hello"}"#,
        ).unwrap();
        match msg {
            SdkMessage::Infer { max_tokens, .. } => assert_eq!(max_tokens, 256),
            _ => panic!("expected Infer variant"),
        }
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
