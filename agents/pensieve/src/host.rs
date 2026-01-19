//! Host function declarations for Extism PDK
//!
//! These functions are provided by the host runtime and enable
//! the WASM plugin to interact with external services.

use extism_pdk::*;

#[host_fn]
extern "ExtismHost" {
    /// Call a local LLM for inference
    pub fn infer(model: String, prompt: String) -> String;

    /// Read a file from the persistence layer (ADR 005)
    pub fn get_file(path: String) -> Vec<u8>;

    /// Write a file to the persistence layer (ADR 005)
    pub fn put_file(path: String, content: Vec<u8>) -> String;

    /// Generate embeddings for text (ADR 005)
    pub fn embed(model: String, text: String) -> String;

    /// Store an embedding vector with metadata (ADR 005)
    pub fn store_embedding(key: String, vector: String, metadata: String) -> String;
}
