//! WasmRuntime - executes WASM agents in response to queue messages.
//!
//! The runtime:
//! - Registers agents to serve
//! - Polls their input queues
//! - Executes WASM on message arrival
//! - Sends responses to reply queues
//! - Runs service workers for infrastructure (storage, inference, embedding)
//!
//! Host functions provided to WASM guests:
//! - send(queue_name, payload) -> message_id
//! - receive(queue_name) -> payload (blocking, processes services while waiting)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, Wasm};

use crate::domain::{Agent, EmbeddingEngine, InferenceEngine, ObjectStorage, VectorStorage};
use crate::queue::{InMemoryQueue, Message, MessageQueue};

use super::services::{
    EmbeddingServiceHandler, InferenceServiceHandler,
    ObjectServiceHandler, VectorServiceHandler,
};

/// Shared service handlers that can be accessed from host functions
struct ServiceHandlers {
    object: ObjectServiceHandler,
    vector: VectorServiceHandler,
    inference: InferenceServiceHandler,
    embedding: EmbeddingServiceHandler,
}

impl ServiceHandlers {
    /// Process one service message if available. Returns true if processed.
    fn tick(&self) -> bool {
        if self.object.tick() { return true; }
        if self.vector.tick() { return true; }
        if self.inference.tick() { return true; }
        if self.embedding.tick() { return true; }
        false
    }
}

pub struct WasmRuntime {
    queue: Arc<InMemoryQueue>,
    agents: HashMap<String, Agent>,
    services: Arc<Mutex<ServiceHandlers>>,
}

impl WasmRuntime {
    pub fn new(queue: Arc<InMemoryQueue>) -> Self {
        let services = ServiceHandlers {
            object: ObjectServiceHandler::new(Arc::clone(&queue)),
            vector: VectorServiceHandler::new(Arc::clone(&queue)),
            inference: InferenceServiceHandler::new(Arc::clone(&queue)),
            embedding: EmbeddingServiceHandler::new(Arc::clone(&queue)),
        };
        Self {
            queue,
            agents: HashMap::new(),
            services: Arc::new(Mutex::new(services)),
        }
    }

    /// Register an agent to be served by this runtime.
    pub fn register(&mut self, agent: Agent) {
        self.agents.insert(agent.name.clone(), agent);
    }

    /// Register storage backends for a namespace (typically agent name).
    pub fn register_storage(
        &mut self,
        namespace: &str,
        object: Arc<dyn ObjectStorage>,
        vector: Arc<dyn VectorStorage>,
    ) {
        let mut services = self.services.lock().unwrap();
        services.object.register(namespace, object);
        services.vector.register(namespace, vector);
    }

    /// Register an inference engine by model name.
    pub fn register_inference(&mut self, model_name: &str, engine: Arc<dyn InferenceEngine>) {
        let mut services = self.services.lock().unwrap();
        services.inference.register(model_name, engine);
    }

    /// Register an embedding engine by model name.
    pub fn register_embedding(&mut self, model_name: &str, engine: Arc<dyn EmbeddingEngine>) {
        let mut services = self.services.lock().unwrap();
        services.embedding.register(model_name, engine);
    }

    pub fn tick(&mut self) {
        // First, try service queues (fast, no WASM execution)
        {
            let services = self.services.lock().unwrap();
            if services.tick() { return; }
        }

        // Then, check each agent's queue for work
        for (name, agent) in &self.agents {
            if let Ok(request) = self.queue.receive(name) {
                // Build host functions with queue and service access
                let functions = [
                    make_send_function(Arc::clone(&self.queue)),
                    make_receive_function(Arc::clone(&self.queue), Arc::clone(&self.services)),
                    make_get_prompts_function(),
                ];

                // Execute WASM
                let wasm_path = agent.code.as_str().strip_prefix("file://").unwrap_or(agent.code.as_str());
                let wasm = Wasm::file(wasm_path);
                let manifest = Manifest::new([wasm])
                    .with_allowed_hosts(["*".to_string()].into_iter());
                let mut plugin = Plugin::new(&manifest, functions, true).unwrap();

                let output = plugin
                    .call::<_, Vec<u8>>("process", &request.payload)
                    .unwrap();

                // Send response to reply queue
                let response = Message::response(output, &request.reply_to, request.id.clone());
                self.queue.send(&request.reply_to, response).unwrap();

                return; // Process one message per tick
            }
        }
    }

    pub fn run(&mut self) {
        loop {
            self.tick();
        }
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
fn make_send_function(queue: Arc<InMemoryQueue>) -> Function {
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

/// Data passed to the receive function - queue and service handlers
struct ReceiveData {
    queue: Arc<InMemoryQueue>,
    services: Arc<Mutex<ServiceHandlers>>,
}

/// Host function: receive(queue_name) -> payload
///
/// Receives a message from a queue. Processes service queues while waiting
/// to avoid deadlock when agent is waiting for a service response.
fn make_receive_function(queue: Arc<InMemoryQueue>, services: Arc<Mutex<ServiceHandlers>>) -> Function {
    Function::new(
        "receive",
        [extism::PTR], // queue_name
        [extism::PTR], // payload (JSON with id, payload, correlation_id)
        UserData::new(ReceiveData { queue, services }),
        |plugin, inputs, outputs, user_data| {
            let queue_name: String = plugin.memory_get_val(&inputs[0])?;

            let data = user_data.get().unwrap();
            let data = data.lock().unwrap();

            // Spin until message available, processing services while waiting
            loop {
                // Check for our message first
                if let Ok(message) = data.queue.receive(&queue_name) {
                    // Return message as JSON
                    let response = serde_json::json!({
                        "id": message.id.to_string(),
                        "payload": String::from_utf8_lossy(&message.payload),
                        "correlation_id": message.correlation_id.map(|id| id.to_string()),
                    });
                    return write_output(plugin, outputs, response.to_string().as_bytes());
                }

                // Process service queues while waiting
                if let Ok(services) = data.services.try_lock() {
                    services.tick();
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
    use base64::Engine as _;
    use crate::queue::{InMemoryQueue, Message, MessageQueue};
    use crate::storage::dispatch::{in_memory_storage, open_object_storage, open_vector_storage};
    use crate::inference::InMemoryInference;
    use crate::embedding::InMemoryEmbedding;
    use std::path::Path;

    fn load_test_agent(name: &str) -> Agent {
        let path = Path::new("tests/fixtures/agents").join(name);
        Agent::load(&path).unwrap()
    }

    #[test]
    fn runtime_registers_agent() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut runtime = WasmRuntime::new(queue);

        let agent = load_test_agent("reverse-agent");
        runtime.register(agent);

        assert!(runtime.agents.contains_key("reverse-agent"));
    }

    #[test]
    fn tick_processes_message() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut runtime = WasmRuntime::new(Arc::clone(&queue));

        // Register agent
        let agent = load_test_agent("reverse-agent");
        runtime.register(agent);

        // Send message to agent's queue
        let reply_queue = "test-reply";
        let request = Message::request(b"hello".to_vec(), reply_queue);
        let request_id = request.id.clone();
        queue.send("reverse-agent", request).unwrap();

        // Tick should process it
        runtime.tick();

        // Response should be on reply queue
        let response = queue.receive(reply_queue).unwrap();
        assert_eq!(response.correlation_id, Some(request_id));
        assert_eq!(String::from_utf8(response.payload).unwrap(), "olleh");
    }

    #[test]
    fn tick_processes_service_messages() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut runtime = WasmRuntime::new(Arc::clone(&queue));

        // Register storage for a namespace
        let storage = in_memory_storage();
        let object = open_object_storage(&storage).unwrap();
        let vector = open_vector_storage(&storage).unwrap();
        runtime.register_storage("test-agent", object, vector);

        // Send a kv-put message (simulating what a WASM agent would do)
        let put_payload = serde_json::json!({
            "namespace": "test-agent",
            "path": "/notes/hello.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"hello world")
        });
        let put_msg = Message::request(
            serde_json::to_vec(&put_payload).unwrap(),
            "reply-queue",
        );
        queue.send("kv-put", put_msg).unwrap();

        // Tick should process the service message
        runtime.tick();

        // Check response
        let response = queue.receive("reply-queue").unwrap();
        assert_eq!(response.payload, b"ok");

        // Now send a kv-get to verify it was stored
        let get_payload = serde_json::json!({
            "namespace": "test-agent",
            "path": "/notes/hello.txt"
        });
        let get_msg = Message::request(
            serde_json::to_vec(&get_payload).unwrap(),
            "reply-queue",
        );
        queue.send("kv-get", get_msg).unwrap();

        runtime.tick();

        let response = queue.receive("reply-queue").unwrap();
        assert_eq!(response.payload, b"hello world");
    }

    #[test]
    fn tick_processes_inference_messages() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut runtime = WasmRuntime::new(Arc::clone(&queue));

        // Register inference engine
        let engine = Arc::new(InMemoryInference::new("The answer is 42."));
        runtime.register_inference("test-model", engine);

        // Send infer message
        let infer_payload = serde_json::json!({
            "model": "test-model",
            "prompt": "What is the meaning of life?"
        });
        let infer_msg = Message::request(
            serde_json::to_vec(&infer_payload).unwrap(),
            "reply-queue",
        );
        queue.send("infer", infer_msg).unwrap();

        runtime.tick();

        let response = queue.receive("reply-queue").unwrap();
        assert_eq!(String::from_utf8(response.payload).unwrap(), "The answer is 42.");
    }

    #[test]
    fn tick_processes_embedding_messages() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut runtime = WasmRuntime::new(Arc::clone(&queue));

        // Register embedding engine with canned 768-dim vector
        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let engine = Arc::new(InMemoryEmbedding::new(canned.clone()));
        runtime.register_embedding("test-model", engine);

        // Send embed message
        let embed_payload = serde_json::json!({
            "model": "test-model",
            "text": "hello world"
        });
        let embed_msg = Message::request(
            serde_json::to_vec(&embed_payload).unwrap(),
            "reply-queue",
        );
        queue.send("embed", embed_msg).unwrap();

        runtime.tick();

        let response = queue.receive("reply-queue").unwrap();
        let vector: Vec<f32> = serde_json::from_slice(&response.payload).unwrap();
        assert_eq!(vector.len(), 768);
        assert_eq!(vector, canned);
    }
}
