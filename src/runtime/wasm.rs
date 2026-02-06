//! WasmRuntime - executes WASM agents in response to queue messages.
//!
//! The runtime:
//! - Registers agents to serve
//! - Polls their input queues
//! - Executes WASM on message arrival
//! - Sends responses to reply queues
//!
//! Agent wiring (validation, routing, request lifecycle) lives in
//! `SendFunctionData` (in `send.rs`). Extism-specific plumbing lives in `wasm_plugin`.

use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::domain::{ObjectStorageType, Registry, ResourceId, Runtime, RuntimeType, VectorStorageType};
use crate::queue::{
    ExpectsReply, InvokeMessage, MessageQueue, SequenceCounter,
};

use super::send::SendFunctionData;
use super::wasm_plugin;

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
            // Use canonical routing key (must match send_invoke subject)
            let routing_key = crate::queue::agent_routing_key(&agent.id);
            if let Ok((invoke, ack)) = self.queue.receive_invoke(&routing_key) {
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
                    let send_data = SendFunctionData {
                        queue,
                        invoke: invoke_for_send,
                        kv_backend,
                        vec_backend,
                        sequence: SequenceCounter::new(),
                    };
                    wasm_plugin::run_plugin(&wasm_path, send_data, &payload)
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
