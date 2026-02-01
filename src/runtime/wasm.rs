//! WasmRuntime - executes WASM agents in response to queue messages.
//!
//! The runtime:
//! - Registers agents to serve
//! - Polls their input queues
//! - Executes WASM on message arrival
//! - Sends responses to reply queues
//!
//! Host functions provided to WASM guests:
//! - send(queue_name, payload) -> message_id
//! - receive(queue_name) -> payload (blocks until message available)

use std::collections::HashMap;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, Wasm};

use crate::domain::{Agent, Runtime};
use crate::queue::{Message, MessageId, MessageQueue};

/// Tracks a WASM execution running in a background thread.
struct RunningTask {
    handle: JoinHandle<Vec<u8>>,
    reply_to: String,
    request_id: MessageId,
}

pub struct WasmRuntime {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    agents: HashMap<String, Agent>,
    running: Option<RunningTask>,
}

impl WasmRuntime {
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>) -> Self {
        Self {
            queue,
            agents: HashMap::new(),
            running: None,
        }
    }
}

impl Runtime for WasmRuntime {
    fn register(&mut self, agent: Agent) {
        self.agents.insert(agent.name.clone(), agent);
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
        for (name, agent) in &self.agents {
            if let Ok(request) = self.queue.receive(name) {
                // Prepare data for the thread
                let queue = Arc::clone(&self.queue);
                let wasm_path = agent
                    .code
                    .as_str()
                    .strip_prefix("file://")
                    .unwrap_or(agent.code.as_str())
                    .to_string();
                let payload = request.payload.clone();

                // Spawn WASM execution in background thread
                let handle = thread::spawn(move || {
                    let functions = [
                        make_send_function(Arc::clone(&queue)),
                        make_receive_function(Arc::clone(&queue)),
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

/// Host function: send(queue_name, payload) -> message_id
///
/// Sends a message to a queue. Returns the message ID for correlation.
fn make_send_function(queue: Arc<dyn MessageQueue + Send + Sync>) -> Function {
    Function::new(
        "send",
        [extism::PTR, extism::PTR, extism::PTR], // queue_name, payload, reply_to
        [extism::PTR],                            // message_id
        UserData::new(queue),
        |plugin, inputs, outputs, user_data| {
            let queue_name: String = plugin.memory_get_val(&inputs[0])?;
            let payload: Vec<u8> = plugin.memory_get_val(&inputs[1])?;
            let reply_to: String = plugin.memory_get_val(&inputs[2])?;

            let queue = user_data.get().unwrap();
            let queue = queue.lock().unwrap();

            let message = Message::request(payload, &reply_to);
            let message_id = message.id.to_string();

            let result = queue.send(&queue_name, message);

            let response = match result {
                Ok(()) => message_id,
                Err(e) => format!("[error] {}", e),
            };

            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

/// Host function: receive(queue_name) -> payload
///
/// Receives a message from a queue. Blocks until a message is available.
/// The caller is responsible for ensuring service workers are running
/// in parallel to avoid deadlock.
fn make_receive_function(queue: Arc<dyn MessageQueue + Send + Sync>) -> Function {
    Function::new(
        "receive",
        [extism::PTR], // queue_name
        [extism::PTR], // payload (JSON with id, payload, correlation_id)
        UserData::new(queue),
        |plugin, inputs, outputs, user_data| {
            let queue_name: String = plugin.memory_get_val(&inputs[0])?;

            let queue = user_data.get().unwrap();
            let queue = queue.lock().unwrap();

            // Spin until message available
            loop {
                if let Ok(message) = queue.receive(&queue_name) {
                    // Return message as JSON
                    let response = serde_json::json!({
                        "id": message.id.to_string(),
                        "payload": String::from_utf8_lossy(&message.payload),
                        "correlation_id": message.correlation_id.map(|id| id.to_string()),
                    });
                    return write_output(plugin, outputs, response.to_string().as_bytes());
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
    use crate::queue::{InMemoryQueue, Message, MessageQueue};
    use std::path::Path;

    fn load_test_agent(name: &str) -> Agent {
        let path = Path::new("tests/fixtures/agents").join(name);
        Agent::load(&path).unwrap()
    }

    #[test]
    fn runtime_registers_agent() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let mut runtime = WasmRuntime::new(queue);

        let agent = load_test_agent("reverse-agent");
        runtime.register(agent);

        assert!(runtime.agents.contains_key("reverse-agent"));
    }

    #[test]
    fn tick_processes_message() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let mut runtime = WasmRuntime::new(Arc::clone(&queue));

        // Register agent
        let agent = load_test_agent("reverse-agent");
        runtime.register(agent);

        // Send message to agent's queue
        let reply_queue = "test-reply";
        let request = Message::request(b"hello".to_vec(), reply_queue);
        let request_id = request.id.clone();
        queue.send("reverse-agent", request).unwrap();

        // First tick spawns the WASM thread
        assert!(runtime.tick());

        // Keep ticking until the task completes and response is sent
        loop {
            if runtime.tick() {
                break; // Task completed
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Response should be on reply queue
        let response = queue.receive(reply_queue).unwrap();
        assert_eq!(response.correlation_id, Some(request_id));
        assert_eq!(String::from_utf8(response.payload).unwrap(), "olleh");
    }
}
