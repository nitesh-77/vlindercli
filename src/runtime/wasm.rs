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

use std::collections::HashMap;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, Wasm};

use crate::domain::{Agent, ResourceId, Runtime, RuntimeType};
use crate::queue::{Message, MessageId, MessageQueue};

/// Tracks a WASM execution running in a background thread.
struct RunningTask {
    handle: JoinHandle<Vec<u8>>,
    reply_to: String,
    request_id: MessageId,
}

pub struct WasmRuntime {
    id: ResourceId,
    queue: Arc<dyn MessageQueue + Send + Sync>,
    agents: HashMap<ResourceId, Agent>,
    running: Option<RunningTask>,
}

impl WasmRuntime {
    pub fn new(registry_id: &ResourceId, queue: Arc<dyn MessageQueue + Send + Sync>) -> Self {
        let id = ResourceId::new(format!(
            "{}/runtimes/{}",
            registry_id.as_str(),
            RuntimeType::Wasm.as_str()
        ));
        Self {
            id,
            queue,
            agents: HashMap::new(),
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

    fn register(&mut self, agent: Agent) {
        self.agents.insert(agent.id.clone(), agent);
    }

    fn tick(&mut self) -> bool {
        // First, check if a running task completed
        if let Some(task) = self.running.take() {
            if task.handle.is_finished() {
                // Task done - send response
                let output = task.handle.join().unwrap();
                let response = Message::response(output, &task.reply_to, task.request_id);
                self.queue.send(&task.reply_to, response).unwrap();
                return true;
            } else {
                // Still running, put it back
                self.running = Some(task);
                return false; // Can't start new work while one is running
            }
        }

        // No running task, check for new work
        for (agent_id, agent) in &self.agents {
            if let Ok(request) = self.queue.receive(agent_id.as_str()) {
                // Prepare data for the thread
                let queue = Arc::clone(&self.queue);
                let wasm_path = agent
                    .id
                    .path()
                    .unwrap_or(agent.id.as_str())
                    .to_string();
                let agent_id_str = agent_id.as_str().to_string();
                let payload = request.payload.clone();

                // Spawn WASM execution in background thread
                let handle = thread::spawn(move || {
                    let functions = [
                        make_send_function(Arc::clone(&queue), agent_id_str),
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
                    reply_to: request.reply_to.clone(),
                    request_id: request.id,
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
    agent_id: String,
}

/// Host function: send(payload) -> response
///
/// Synchronous request/response. The runtime:
/// - Extracts `op` from payload to determine target queue
/// - Injects `agent_id` for storage isolation
/// - Handles reply queue management
/// - Returns the response payload directly
fn make_send_function(queue: Arc<dyn MessageQueue + Send + Sync>, agent_id: String) -> Function {
    let data = SendFunctionData { queue, agent_id };

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
                obj.insert("agent_id".to_string(), serde_json::Value::String(data.agent_id.clone()));
            }

            // Generate reply queue
            let reply_to = format!("{}-reply-{}", data.agent_id, uuid::Uuid::new_v4());

            // Send request
            let enriched_payload = serde_json::to_vec(&json)
                .map_err(|e| extism::Error::msg(format!("serialize error: {}", e)))?;
            let message = Message::request(enriched_payload, &reply_to);

            data.queue.send(&op, message)
                .map_err(|e| extism::Error::msg(format!("send error: {}", e)))?;

            // Wait for response
            loop {
                if let Ok(response) = data.queue.receive(&reply_to) {
                    return write_output(plugin, outputs, &response.payload);
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
    use crate::queue::InMemoryQueue;

    fn test_registry_id() -> ResourceId {
        ResourceId::new("http://test:9000")
    }

    #[test]
    fn runtime_id_format() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let runtime = WasmRuntime::new(&test_registry_id(), queue);

        // Format: <registry_id>/runtimes/<runtime_type>
        assert_eq!(runtime.id().as_str(), "http://test:9000/runtimes/wasm");
        assert_eq!(runtime.runtime_type(), RuntimeType::Wasm);
    }
}
