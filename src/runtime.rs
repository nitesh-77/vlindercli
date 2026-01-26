use std::path::Path;
use std::sync::Arc;

use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, Wasm};

use crate::config;
use crate::domain::Agent;
use crate::inference::{load_embedding_engine, load_engine};
use crate::storage::Storage;

// ============================================================================
// Error Handling
// ============================================================================

/// Errors that can occur in host functions
#[derive(Debug)]
pub enum HostError {
    ModelNotDeclared(String),
    ModelLoad { model: String, reason: String },
    Inference(String),
    Embedding(String),
    Storage(String),
    Json(String),
    FileNotFound,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::ModelNotDeclared(name) => write!(f, "model '{}' not declared by agent", name),
            HostError::ModelLoad { model, reason } => write!(f, "failed to load '{}': {}", model, reason),
            HostError::Inference(e) => write!(f, "{}", e),
            HostError::Embedding(e) => write!(f, "{}", e),
            HostError::Storage(e) => write!(f, "{}", e),
            HostError::Json(e) => write!(f, "{}", e),
            HostError::FileNotFound => write!(f, "file not found"),
        }
    }
}

type HostResult<T> = Result<T, HostError>;

/// Convert a HostResult to a response string
fn to_response(result: HostResult<String>) -> String {
    match result {
        Ok(s) => s,
        Err(e) => format!("[error] {}", e),
    }
}

/// Convert a HostResult<Vec<u8>> to response bytes
fn to_response_bytes(result: HostResult<Vec<u8>>) -> Vec<u8> {
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

    /// Execute an agent at the given path with the provided input.
    ///
    /// Loads the agent manifest, resolves paths, and runs the WASM plugin.
    pub fn execute(&self, agent_path: &Path, input: &str) -> String {
        // Load agent from path
        let agent = match Agent::load(agent_path) {
            Ok(a) => a,
            Err(e) => return format!("[error] failed to load agent: {:?}", e),
        };

        // Ensure default mount directory exists
        let mnt_path = config::agent_mnt_path(&agent.name);
        if !mnt_path.exists() {
            if let Err(e) = std::fs::create_dir_all(&mnt_path) {
                tracing::warn!("Failed to create mount directory: {}", e);
            }
        }

        // Open storage for this agent
        let storage = match Storage::open(&agent.name) {
            Ok(s) => Arc::new(s),
            Err(e) => return format!("[error] failed to open storage: {}", e),
        };

        let functions = [
            make_get_manifest_function(agent.clone()),
            make_infer_function(agent.clone()),
            make_embed_function(agent.clone()),
            make_put_file_function(storage.clone()),
            make_get_file_function(storage.clone()),
            make_delete_file_function(storage.clone()),
            make_list_files_function(storage.clone()),
            make_store_embedding_function(storage.clone()),
            make_search_by_vector_function(storage.clone()),
        ];

        // Build WASM manifest with allowed paths from agent mounts
        let wasm = Wasm::file(&agent.wasm_path);
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

fn make_get_manifest_function(agent: Agent) -> Function {
    Function::new(
        "get_manifest",
        [],
        [extism::PTR],
        UserData::new(agent),
        |plugin, _inputs, outputs, user_data| {
            let agent = user_data.get().unwrap();
            let agent = agent.lock().unwrap();
            write_output(plugin, outputs, agent.manifest().as_bytes())
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

            let response = to_response(do_infer(&agent, &model_name, &prompt));
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

            let response = to_response(do_embed(&agent, &model_name, &text));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_put_file_function(storage: Arc<Storage>) -> Function {
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

            let response = to_response(do_put_file(&storage, &path, &content));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_get_file_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "get_file",
        [extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let path: String = plugin.memory_get_val(&inputs[0])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = to_response_bytes(do_get_file(&storage, &path));
            write_output(plugin, outputs, &response)
        },
    )
}

fn make_delete_file_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "delete_file",
        [extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let path: String = plugin.memory_get_val(&inputs[0])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = to_response(do_delete_file(&storage, &path));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_list_files_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "list_files",
        [extism::PTR],
        [extism::PTR],
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let dir_path: String = plugin.memory_get_val(&inputs[0])?;
            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = to_response(do_list_files(&storage, &dir_path));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_store_embedding_function(storage: Arc<Storage>) -> Function {
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

            let response = to_response(do_store_embedding(&storage, &key, &vector_json, &metadata));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn make_search_by_vector_function(storage: Arc<Storage>) -> Function {
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

            let response = to_response(do_search_by_vector(&storage, &query_json, &limit_str));
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

// ============================================================================
// Business Logic (separated from host function boilerplate)
// ============================================================================

fn do_infer(agent: &Agent, model_name: &str, prompt: &str) -> HostResult<String> {
    if !agent.has_model(model_name) {
        return Err(HostError::ModelNotDeclared(model_name.to_string()));
    }

    let engine = load_engine(model_name)
        .map_err(|e| HostError::ModelLoad {
            model: model_name.to_string(),
            reason: e,
        })?;

    engine.infer(prompt, 256)
        .map_err(HostError::Inference)
}

fn do_embed(agent: &Agent, model_name: &str, text: &str) -> HostResult<String> {
    if !agent.has_model(model_name) {
        return Err(HostError::ModelNotDeclared(model_name.to_string()));
    }

    let engine = load_embedding_engine(model_name)
        .map_err(|e| HostError::ModelLoad {
            model: model_name.to_string(),
            reason: e,
        })?;

    let vec = engine.embed(text)
        .map_err(HostError::Embedding)?;

    serde_json::to_string(&vec)
        .map_err(|e| HostError::Json(e.to_string()))
}

fn do_put_file(storage: &Storage, path: &str, content: &[u8]) -> HostResult<String> {
    storage.put_file(path, content)
        .map_err(|e| HostError::Storage(e.to_string()))?;
    Ok("ok".to_string())
}

fn do_get_file(storage: &Storage, path: &str) -> HostResult<Vec<u8>> {
    match storage.get_file(path) {
        Ok(Some(content)) => Ok(content),
        Ok(None) => Err(HostError::FileNotFound),
        Err(e) => Err(HostError::Storage(e.to_string())),
    }
}

fn do_delete_file(storage: &Storage, path: &str) -> HostResult<String> {
    match storage.delete_file(path) {
        Ok(true) => Ok("ok".to_string()),
        Ok(false) => Ok("not_found".to_string()),
        Err(e) => Err(HostError::Storage(e.to_string())),
    }
}

fn do_list_files(storage: &Storage, dir_path: &str) -> HostResult<String> {
    let files = storage.list_files(dir_path)
        .map_err(|e| HostError::Storage(e.to_string()))?;

    serde_json::to_string(&files)
        .map_err(|e| HostError::Json(e.to_string()))
}

fn do_store_embedding(storage: &Storage, key: &str, vector_json: &str, metadata: &str) -> HostResult<String> {
    let vector: Vec<f32> = serde_json::from_str(vector_json)
        .map_err(|e| HostError::Json(format!("invalid vector JSON: {}", e)))?;

    storage.store_embedding(key, &vector, metadata)
        .map_err(|e| HostError::Storage(e.to_string()))?;

    Ok("ok".to_string())
}

fn do_search_by_vector(storage: &Storage, query_json: &str, limit_str: &str) -> HostResult<String> {
    let query_vector: Vec<f32> = serde_json::from_str(query_json)
        .map_err(|e| HostError::Json(format!("invalid vector JSON: {}", e)))?;

    let limit = limit_str.parse::<u32>().unwrap_or(10);

    let results = storage.search_by_vector(&query_vector, limit)
        .map_err(|e| HostError::Storage(e.to_string()))?;

    let formatted: Vec<serde_json::Value> = results.iter()
        .map(|(key, metadata, distance)| {
            serde_json::json!({
                "key": key,
                "metadata": metadata,
                "distance": distance
            })
        })
        .collect();

    serde_json::to_string(&formatted)
        .map_err(|e| HostError::Json(e.to_string()))
}
