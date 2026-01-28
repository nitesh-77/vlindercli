//! WasmRuntime - executes WASM agents in response to queue messages.
//!
//! The runtime:
//! - Registers agents to serve
//! - Polls their input queues
//! - Executes WASM on message arrival
//! - Sends responses to reply queues

use std::collections::HashMap;
use std::sync::Arc;

use extism::{Manifest, Plugin, Wasm};

use crate::domain::Agent;
use crate::queue::{InMemoryQueue, Message, MessageQueue};

pub struct WasmRuntime {
    queue: Arc<InMemoryQueue>,
    agents: HashMap<String, Agent>,
}

impl WasmRuntime {
    pub fn new(queue: Arc<InMemoryQueue>) -> Self {
        Self {
            queue,
            agents: HashMap::new(),
        }
    }

    pub fn register(&mut self, agent: Agent) {
        self.agents.insert(agent.name.clone(), agent);
    }

    pub fn tick(&mut self) {
        // Check each agent's queue for work
        for (name, agent) in &self.agents {
            if let Ok(request) = self.queue.receive(name) {
                // Execute WASM
                let wasm_path = agent.code.as_str().strip_prefix("file://").unwrap_or(agent.code.as_str());
                let wasm = Wasm::file(wasm_path);
                let manifest = Manifest::new([wasm]);
                let mut plugin = Plugin::new(&manifest, [], true).unwrap();

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
}
