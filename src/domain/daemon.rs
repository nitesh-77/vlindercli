//! Daemon - owns and coordinates all system components.
//!
//! The Daemon is the single owner of:
//! - Registry (agent/model catalog)
//! - Runtime (agent execution)
//! - Provider (service workers)
//! - Queue (message passing)
//!
//! Harness becomes a thin client that talks to Daemon.

use std::path::Path;
use std::sync::Arc;

use crate::domain::{Agent, Model, ModelType, Provider, Runtime};
use crate::embedding::{open_embedding_engine, InMemoryEmbedding};
use crate::inference::{open_inference_engine, InMemoryInference};
use crate::queue::{InMemoryQueue, Message, MessageId, MessageQueue};
use crate::runtime::WasmRuntime;
use crate::storage::dispatch::{in_memory_storage, open_object_storage, open_vector_storage};

/// Unique identifier for a registered agent.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentId(String);

impl AgentId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The daemon coordinates all system components.
pub struct Daemon {
    // Components
    queue: Arc<InMemoryQueue>,
    provider: Provider,
    runtime: WasmRuntime,

    // Reply queue for collecting responses
    reply_queue: String,
}

impl Daemon {
    /// Create a new daemon with all components initialized.
    pub fn new() -> Self {
        let queue = Arc::new(InMemoryQueue::new());
        let provider = Provider::new(queue.clone());
        let runtime = WasmRuntime::new(queue.clone());

        Self {
            queue,
            provider,
            runtime,
            reply_queue: "daemon-replies".to_string(),
        }
    }

    /// Register an agent from TOML manifest content.
    ///
    /// The manifest should contain resolved absolute URIs for `id` and model paths.
    /// Returns the agent ID for later invocation.
    pub fn register_agent(&mut self, manifest_toml: &str) -> Result<AgentId, String> {
        let agent = Agent::from_toml(manifest_toml)
            .map_err(|e| format!("failed to parse agent manifest: {:?}", e))?;

        let agent_id = AgentId::new(&agent.name);

        // Register storage on Provider
        let storage = in_memory_storage();
        let object = open_object_storage(&storage).expect("in-memory storage always succeeds");
        let vector = open_vector_storage(&storage).expect("in-memory storage always succeeds");
        self.provider.register_object(&agent.name, object);
        self.provider.register_vector(&agent.name, vector);

        // Register models on Provider
        for (model_name, model_uri) in &agent.requirements.models {
            self.register_model(model_name, model_uri.as_str());
        }

        // Register agent on Runtime
        self.runtime.register(agent);

        Ok(agent_id)
    }

    /// Register a model by loading its manifest and creating the appropriate engine.
    fn register_model(&mut self, model_name: &str, model_uri: &str) {
        let manifest_path = model_uri
            .strip_prefix("file://")
            .unwrap_or(model_uri);

        let model = match Model::load(Path::new(manifest_path)) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[warning] Failed to load model {}: {}", model_name, e);
                return;
            }
        };

        match model.model_type {
            ModelType::Inference => {
                match open_inference_engine(&model) {
                    Ok(engine) => {
                        eprintln!("[info] Loaded inference model: {}", model_name);
                        self.provider.register_inference(model_name, engine);
                    }
                    Err(e) => {
                        eprintln!("[warning] Using placeholder inference for {}: {}", model_name, e);
                        let engine = Arc::new(InMemoryInference::new("[inference placeholder]"));
                        self.provider.register_inference(model_name, engine);
                    }
                }
            }
            ModelType::Embedding => {
                match open_embedding_engine(&model) {
                    Ok(engine) => {
                        eprintln!("[info] Loaded embedding model: {}", model_name);
                        self.provider.register_embedding(model_name, engine);
                    }
                    Err(e) => {
                        eprintln!("[warning] Using placeholder embedding for {}: {}", model_name, e);
                        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
                        let engine = Arc::new(InMemoryEmbedding::new(canned));
                        self.provider.register_embedding(model_name, engine);
                    }
                }
            }
        }
    }

    /// Send input to an agent.
    ///
    /// Returns a message ID for polling the response.
    pub fn send(&self, agent_id: &AgentId, input: &str) -> Result<MessageId, String> {
        let agent_queue = agent_id.as_str();
        let message = Message::request(input.as_bytes().to_vec(), self.reply_queue.clone());
        let message_id = message.id.clone();

        self.queue.send(agent_queue, message)
            .map_err(|e| e.to_string())?;

        Ok(message_id)
    }

    /// Poll for a response to a previous request.
    ///
    /// Returns `Some(output)` if response ready, `None` if still processing.
    pub fn poll(&self, message_id: &MessageId) -> Result<Option<String>, String> {
        // Try to receive - treat QueueNotFound as "no message yet"
        if let Ok(response) = self.queue.receive(&self.reply_queue) {
            let output = String::from_utf8(response.payload)
                .map_err(|e| e.to_string())?;

            if response.correlation_id.as_ref() == Some(message_id) {
                return Ok(Some(output));
            }
            // Not our message - for now, drop it (simple single-request model)
        }
        Ok(None)
    }

    /// Drive the control loop - tick all components.
    pub fn tick(&mut self) {
        let _ = self.provider.tick();
        let _ = self.runtime.tick();
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    /// Build a manifest TOML with absolute paths for the reverse-agent fixture.
    fn reverse_agent_manifest() -> String {
        let wasm_path = fixture_path("agents/reverse-agent/agent.wasm");
        format!(
            r#"
            name = "reverse-agent"
            description = "Reverses input string"
            id = "file://{}"

            [requirements]
            services = []
            "#,
            wasm_path.display()
        )
    }

    #[test]
    fn daemon_register_send_poll() {
        let mut daemon = Daemon::new();

        // Register agent with TOML content (absolute paths)
        let manifest = reverse_agent_manifest();
        let agent_id = daemon.register_agent(&manifest).unwrap();
        assert_eq!(agent_id.as_str(), "reverse-agent");

        // Send input
        let msg_id = daemon.send(&agent_id, "hello").unwrap();

        // Tick until response
        let output = loop {
            daemon.tick();
            if let Some(output) = daemon.poll(&msg_id).unwrap() {
                break output;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };

        assert_eq!(output, "olleh");
    }
}
