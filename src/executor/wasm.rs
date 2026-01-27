//! WASM executor using Extism.

use std::sync::Arc;

use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, Wasm};

use crate::domain::{Agent, Model, ModelType};
use crate::loader;
use crate::services::{object_storage, vector_storage};
use crate::storage::{ObjectStorage, VectorStorage};

use super::{EmbeddingFactory, Executor, InferenceFactory};

// ============================================================================
// WasmExecutor
// ============================================================================

pub struct WasmExecutor {
    inference_factory: InferenceFactory,
    embedding_factory: EmbeddingFactory,
}

impl WasmExecutor {
    pub fn new<I, E>(inference_factory: I, embedding_factory: E) -> Self
    where
        I: Fn(&Model) -> Result<Arc<dyn crate::inference::InferenceEngine>, String> + Send + Sync + 'static,
        E: Fn(&Model) -> Result<Arc<dyn crate::embedding::EmbeddingEngine>, String> + Send + Sync + 'static,
    {
        Self {
            inference_factory: Arc::new(inference_factory),
            embedding_factory: Arc::new(embedding_factory),
        }
    }
}

impl Executor for WasmExecutor {
    fn execute(
        &self,
        agent: &Agent,
        input: &str,
        object_storage: Arc<dyn ObjectStorage>,
        vector_storage: Arc<dyn VectorStorage>,
    ) -> Result<String, String> {
        // Build host functions
        let functions = [
            make_get_prompts_function(agent.clone()),
            make_infer_function(agent.clone(), &self.inference_factory),
            make_embed_function(agent.clone(), &self.embedding_factory),
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
            let host_key = if mount.readonly {
                format!("ro:{}", mount.host_path.display())
            } else {
                mount.host_path.display().to_string()
            };
            manifest = manifest.with_allowed_path(host_key, &mount.guest_path);
        }

        // Create and run plugin
        let mut plugin = Plugin::new(&manifest, functions, true)
            .map_err(|e| format!("failed to create plugin: {}", e))?;

        let bytes = plugin.call::<_, Vec<u8>>("process", input)
            .map_err(|e| format!("plugin execution failed: {}", e))?;

        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn to_response<T: AsRef<str>, E: std::fmt::Display>(result: Result<T, E>) -> String {
    match result {
        Ok(s) => s.as_ref().to_string(),
        Err(e) => format!("[error] {}", e),
    }
}

fn to_response_bytes<E: std::fmt::Display>(result: Result<Vec<u8>, E>) -> Vec<u8> {
    match result {
        Ok(bytes) => bytes,
        Err(e) => format!("[error] {}", e).into_bytes(),
    }
}

fn write_output(plugin: &mut CurrentPlugin, outputs: &mut [Val], response: &[u8]) -> Result<(), extism::Error> {
    let handle = plugin.memory_new(response)?;
    outputs[0] = Val::I64(handle.offset() as i64);
    Ok(())
}

// ============================================================================
// Host Function Builders
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

/// Helper struct to hold agent and factory for infer host function.
struct InferContext {
    agent: Agent,
    factory: InferenceFactory,
}

fn make_infer_function(agent: Agent, factory: &InferenceFactory) -> Function {
    let context = InferContext {
        agent,
        factory: Arc::clone(factory),
    };

    Function::new(
        "infer",
        [extism::PTR, extism::PTR],
        [extism::PTR],
        UserData::new(context),
        |plugin, inputs, outputs, user_data| {
            let model_name: String = plugin.memory_get_val(&inputs[0])?;
            let prompt: String = plugin.memory_get_val(&inputs[1])?;
            let context = user_data.get().unwrap();
            let context = context.lock().unwrap();

            let response = run_infer(&context.agent, &model_name, &prompt, &context.factory);
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn run_infer(agent: &Agent, model_name: &str, prompt: &str, factory: &InferenceFactory) -> String {
    let result = (|| -> Result<String, String> {
        let model_uri = agent.model_uri(model_name)
            .ok_or_else(|| format!("model '{}' not declared by agent", model_name))?;

        let resolved_uri = resolve_uri(model_uri, &agent.agent_dir);
        let model = loader::load_model(&resolved_uri)
            .map_err(|e| format!("failed to load '{}': {}", model_name, e))?;

        if model.model_type != ModelType::Inference {
            return Err(format!(
                "model '{}' has type {:?} but inference was expected",
                model_name, model.model_type
            ));
        }

        let engine = factory(&model)?;
        engine.infer(prompt, 256)
    })();

    to_response(result)
}

/// Helper struct to hold agent and factory for embed host function.
struct EmbedContext {
    agent: Agent,
    factory: EmbeddingFactory,
}

fn make_embed_function(agent: Agent, factory: &EmbeddingFactory) -> Function {
    let context = EmbedContext {
        agent,
        factory: Arc::clone(factory),
    };

    Function::new(
        "embed",
        [extism::PTR, extism::PTR],
        [extism::PTR],
        UserData::new(context),
        |plugin, inputs, outputs, user_data| {
            let model_name: String = plugin.memory_get_val(&inputs[0])?;
            let text: String = plugin.memory_get_val(&inputs[1])?;
            let context = user_data.get().unwrap();
            let context = context.lock().unwrap();

            let response = run_embed(&context.agent, &model_name, &text, &context.factory);
            write_output(plugin, outputs, response.as_bytes())
        },
    )
}

fn run_embed(agent: &Agent, model_name: &str, text: &str, factory: &EmbeddingFactory) -> String {
    let result = (|| -> Result<String, String> {
        let model_uri = agent.model_uri(model_name)
            .ok_or_else(|| format!("model '{}' not declared by agent", model_name))?;

        let resolved_uri = resolve_uri(model_uri, &agent.agent_dir);
        let model = loader::load_model(&resolved_uri)
            .map_err(|e| format!("failed to load '{}': {}", model_name, e))?;

        if model.model_type != ModelType::Embedding {
            return Err(format!(
                "model '{}' has type {:?} but embedding was expected",
                model_name, model.model_type
            ));
        }

        let engine = factory(&model)?;
        let vec = engine.embed(text)?;
        serde_json::to_string(&vec).map_err(|e| e.to_string())
    })();

    to_response(result)
}

fn resolve_uri(uri: &str, base_dir: &std::path::Path) -> String {
    if let Some(path) = uri.strip_prefix("file://") {
        if path.starts_with("./") || !std::path::Path::new(path).is_absolute() {
            let clean_path = path.strip_prefix("./").unwrap_or(path);
            let resolved = base_dir.join(clean_path);
            return format!("file://{}", resolved.display());
        }
    }
    uri.to_string()
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
