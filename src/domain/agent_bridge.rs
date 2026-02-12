//! AgentBridge — platform services contract for agent containers (ADR 074).
//!
//! Defines what platform services are available to agents. Transport layers
//! (HTTP, gRPC) deserialize requests and call these typed methods.
//! Implementations route calls to the appropriate backend.

use serde::{Deserialize, Serialize};

/// Platform services exposed to agent containers.
///
/// Each method maps to one SDK operation. Transport layers parse agent
/// requests and call these methods; the implementation routes them
/// to the correct backend.
pub trait AgentBridge: Send + Sync {
    // -- Object storage --
    fn kv_get(&self, path: &str) -> Result<Vec<u8>, String>;
    fn kv_put(&self, path: &str, content: &str) -> Result<(), String>;
    fn kv_list(&self, prefix: &str) -> Result<Vec<String>, String>;
    fn kv_delete(&self, path: &str) -> Result<bool, String>;

    // -- Vector storage --
    fn vector_store(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String>;
    fn vector_search(&self, vector: &[f32], limit: u32) -> Result<Vec<VectorMatch>, String>;
    fn vector_delete(&self, key: &str) -> Result<bool, String>;

    // -- Inference --
    fn infer(&self, model: &str, prompt: &str, max_tokens: u32) -> Result<String, String>;

    // -- Embedding --
    fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, String>;

    // -- Delegation (ADR 056) --
    fn delegate(&self, target_agent: &str, input: &str) -> Result<String, String>;
    fn wait(&self, handle: &str) -> Result<Vec<u8>, String>;
}

/// A single vector search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMatch {
    pub key: String,
    pub metadata: String,
    pub distance: f32,
}
