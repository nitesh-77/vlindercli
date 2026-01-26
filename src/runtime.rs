use std::sync::Arc;

use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, Wasm};

use crate::config;
use crate::domain::Agent;
use crate::loader;
use crate::services::{embedding, inference, object_storage, vector_storage};
use crate::storage::{ObjectStorage, VectorStorage, open_object_storage, open_vector_storage};

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert a Result to a response string, formatting errors with [error] prefix
fn to_response<T: AsRef<str>, E: std::fmt::Display>(result: Result<T, E>) -> String {
    match result {
        Ok(s) => s.as_ref().to_string(),
        Err(e) => format!("[error] {}", e),
    }
}

/// Convert a Result<Vec<u8>> to response bytes
fn to_response_bytes<E: std::fmt::Display>(result: Result<Vec<u8>, E>) -> Vec<u8> {
    match result {
        Ok(bytes) => bytes,
        Err(e) => format!("[error] {}", e).into_bytes(),
    }
}

/// Write response bytes to plugin memory
fn write_output(plugin: &mut CurrentPlugin, outputs: &mut [Val], response: &[u8]) -> Result<(), extism::Error> {
    let handle = plugin.memory_new(response)?;
    outputs[0] = Val::I64(handle.offset() as i64);
    Ok(())
}

// ============================================================================
// Runtime
// ============================================================================

pub struct Runtime;

impl Runtime {
    pub fn new() -> Self {
        Runtime
    }

    /// Execute an agent identified by URI with the provided input.
    ///
    /// Loads the agent via the loader module and runs the WASM plugin.
    pub fn execute(&self, uri: &str, input: &str) -> String {
        // Load agent via loader
        let agent = match loader::load_agent(uri) {
            Ok(a) => a,
            Err(e) => return format!("[error] failed to load agent: {}", e),
        };

        // Ensure default mount directory exists
        let mnt_path = config::agent_mnt_path(&agent.name);
        if !mnt_path.exists() {
            if let Err(e) = std::fs::create_dir_all(&mnt_path) {
                tracing::warn!("Failed to create mount directory: {}", e);
            }
        }

        // Open storage for this agent
        let object_storage = match open_object_storage(&agent) {
            Ok(s) => s,
            Err(e) => return format!("[error] failed to open object storage: {}", e),
        };

        let vector_storage = match open_vector_storage(&agent) {
            Ok(s) => s,
            Err(e) => return format!("[error] failed to open vector storage: {}", e),
        };

        let functions = [
            make_get_prompts_function(agent.clone()),
            make_infer_function(agent.clone()),
            make_embed_function(agent.clone()),
            make_put_file_function(object_storage.clone()),
            make_get_file_function(object_storage.clone()),
            make_delete_file_function(object_storage.clone()),
            make_list_files_function(object_storage.clone()),
            make_store_embedding_function(vector_storage.clone()),
            make_search_by_vector_function(vector_storage.clone()),
        ];

        // Parse code URI to get the path
        let code_path = agent.code
            .strip_prefix("file://")
            .unwrap_or(&agent.code);

        // Build WASM manifest with allowed paths from agent mounts
        let wasm = Wasm::file(code_path);
        let mut manifest = Manifest::new([wasm]).with_allowed_host("*");

        for mount in &agent.mounts {
            // Extism uses "ro:" prefix for read-only paths
            let host_key = if mount.readonly {
                format!("ro:{}", mount.host_path.display())
            } else {
                mount.host_path.display().to_string()
            };
            manifest = manifest.with_allowed_path(host_key, &mount.guest_path);
        }

        // Create and run plugin
        let mut plugin = match Plugin::new(&manifest, functions, true) {
            Ok(p) => p,
            Err(e) => return format!("[error] failed to create plugin: {}", e),
        };

        match plugin.call::<_, Vec<u8>>("process", input) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => format!("[error] plugin execution failed: {}", e),
        }
    }
}

// ============================================================================
// Host Function Definitions
// ============================================================================

fn make_get_prompts_function(agent: Agent) -> Function {
    Function::new(
        "get_prompts",
        [],
        [extism::PTR],
        UserData::new(agent),
        |plugin, _inputs, outputs, user_data| {
            let agent = user_data.get().unwrap();
            let agent = agent.lock().unwrap();
            let json = serde_json::to_string(&agent.prompts)
                .unwrap_or_else(|_| "null".to_string());
            write_output(plugin, outputs, json.as_bytes())
        },
    )
}

fn make_infer_function(agent: Agent) -> Function {
    Function::new(
        "infer",
        [extism::PTR, extism::PTR],
        [extism::PTR],
        UserData::new(agent),
        |plugin, inputs, outputs, user_data| {
            let model_name: String = plugin.memory_get_val(&inputs[0])?;
            let prompt: String = plugin.memory_get_val(&inputs[1])?;
            let agent = user_data.get().unwrap();
            let agent = agent.lock().unwrap();

            let response = to_response(inference::infer(&agent, &model_name, &prompt));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_embed_function(agent: Agent) -> Function {
    Function::new(
        "embed",
        [extism::PTR, extism::PTR],
        [extism::PTR],
        UserData::new(agent),
        |plugin, inputs, outputs, user_data| {
            let model_name: String = plugin.memory_get_val(&inputs[0])?;
            let text: String = plugin.memory_get_val(&inputs[1])?;
            let agent = user_data.get().unwrap();
            let agent = agent.lock().unwrap();

            let response = to_response(embedding::embed(&agent, &model_name, &text));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_put_file_function(storage: Arc<dyn ObjectStorage>) -> Function {
    Function::new(
        "put_file",
        [extism::PTR, extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let path: String = plugin.memory_get_val(&inputs[0])?;
            let content: Vec<u8> = plugin.memory_get_val(&inputs[1])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = to_response(object_storage::put_file(storage.as_ref(), &path, &content));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_get_file_function(storage: Arc<dyn ObjectStorage>) -> Function {
    Function::new(
        "get_file",
        [extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let path: String = plugin.memory_get_val(&inputs[0])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = to_response_bytes(object_storage::get_file(storage.as_ref(), &path));
            write_output(plugin, outputs, &response)
        },
    )
}

fn make_delete_file_function(storage: Arc<dyn ObjectStorage>) -> Function {
    Function::new(
        "delete_file",
        [extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let path: String = plugin.memory_get_val(&inputs[0])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = to_response(object_storage::delete_file(storage.as_ref(), &path));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_list_files_function(storage: Arc<dyn ObjectStorage>) -> Function {
    Function::new(
        "list_files",
        [extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let dir_path: String = plugin.memory_get_val(&inputs[0])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = to_response(object_storage::list_files(storage.as_ref(), &dir_path));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_store_embedding_function(storage: Arc<dyn VectorStorage>) -> Function {
    Function::new(
        "store_embedding",
        [extism::PTR, extism::PTR, extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let key: String = plugin.memory_get_val(&inputs[0])?;
            let vector_json: String = plugin.memory_get_val(&inputs[1])?;
            let metadata: String = plugin.memory_get_val(&inputs[2])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = to_response(vector_storage::store_embedding(storage.as_ref(), &key, &vector_json, &metadata));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_search_by_vector_function(storage: Arc<dyn VectorStorage>) -> Function {
    Function::new(
        "search_by_vector",
        [extism::PTR, extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let query_json: String = plugin.memory_get_val(&inputs[0])?;
            let limit_str: String = plugin.memory_get_val(&inputs[1])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let limit = limit_str.parse::<u32>().unwrap_or(10);
            let response = to_response(vector_storage::search_by_vector(storage.as_ref(), &query_json, limit));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}
