use crate::domain::Agent;
use crate::inference::{load_embedding_engine, load_engine};
use crate::storage::Storage;
use extism::{Function, UserData};
use std::sync::Arc;

pub struct Runtime;

impl Runtime {
    pub fn new() -> Self {
        Runtime
    }

    pub fn execute(&self, agent: &Agent, input: &str) -> String {
        // Open storage for this agent
        let storage = match Storage::open(&agent.name) {
            Ok(s) => Arc::new(s),
            Err(e) => return format!("[error] failed to open storage: {}", e),
        };

        agent.execute_with_functions(input, [
            make_infer_function(agent.clone()),
            make_embed_function(agent.clone()),
            make_put_file_function(storage.clone()),
            make_get_file_function(storage.clone()),
            make_delete_file_function(storage.clone()),
            make_list_files_function(storage.clone()),
            make_store_embedding_function(storage.clone()),
            make_search_by_vector_function(storage.clone()),
        ])
    }
}

fn make_infer_function(agent: Agent) -> Function {
    Function::new(
        "infer",
        [extism::PTR, extism::PTR],  // model_name, prompt
        [extism::PTR],
        UserData::new(agent),        // attach agent for validation
        |plugin, inputs, outputs, user_data| {
            let model_name: String = plugin.memory_get_val(&inputs[0])?;
            let prompt: String = plugin.memory_get_val(&inputs[1])?;

            let agent = user_data.get().unwrap();
            let agent = agent.lock().unwrap();

            let response = if !agent.has_model(&model_name) {
                format!("[error] model '{}' not declared by agent", model_name)
            } else {
                match load_engine(&model_name) {
                    Ok(engine) => engine.infer(&prompt, 512)
                        .unwrap_or_else(|e| format!("[error] {}", e)),
                    Err(e) => format!("[error] failed to load '{}': {}", model_name, e),
                }
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

fn make_put_file_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "put_file",
        [extism::PTR, extism::PTR],  // path, content
        [extism::PTR],               // returns "ok" or error
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let path: String = plugin.memory_get_val(&inputs[0])?;
            let content: Vec<u8> = plugin.memory_get_val(&inputs[1])?;

            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = match storage.put_file(&path, &content) {
                Ok(()) => "ok".to_string(),
                Err(e) => format!("[error] {}", e),
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

fn make_get_file_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "get_file",
        [extism::PTR],    // path
        [extism::PTR],    // returns content or error
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let path: String = plugin.memory_get_val(&inputs[0])?;

            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response: Vec<u8> = match storage.get_file(&path) {
                Ok(Some(content)) => content,
                Ok(None) => "[error] file not found".as_bytes().to_vec(),
                Err(e) => format!("[error] {}", e).into_bytes(),
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

fn make_delete_file_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "delete_file",
        [extism::PTR],    // path
        [extism::PTR],    // returns "ok" or "not_found"
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let path: String = plugin.memory_get_val(&inputs[0])?;

            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = match storage.delete_file(&path) {
                Ok(true) => "ok".to_string(),
                Ok(false) => "not_found".to_string(),
                Err(e) => format!("[error] {}", e),
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

fn make_list_files_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "list_files",
        [extism::PTR],    // dir_path
        [extism::PTR],    // returns JSON array of paths
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let dir_path: String = plugin.memory_get_val(&inputs[0])?;

            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = match storage.list_files(&dir_path) {
                Ok(files) => serde_json::to_string(&files)
                    .unwrap_or_else(|e| format!("[error] {}", e)),
                Err(e) => format!("[error] {}", e),
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

fn make_embed_function(agent: Agent) -> Function {
    Function::new(
        "embed",
        [extism::PTR, extism::PTR],  // model_name, text
        [extism::PTR],               // returns JSON array of floats
        UserData::new(agent),
        |plugin, inputs, outputs, user_data| {
            let model_name: String = plugin.memory_get_val(&inputs[0])?;
            let text: String = plugin.memory_get_val(&inputs[1])?;

            let agent = user_data.get().unwrap();
            let agent = agent.lock().unwrap();

            let response = if !agent.has_model(&model_name) {
                format!("[error] model '{}' not declared by agent", model_name)
            } else {
                match load_embedding_engine(&model_name) {
                    Ok(engine) => match engine.embed(&text) {
                        Ok(vec) => serde_json::to_string(&vec)
                            .unwrap_or_else(|e| format!("[error] {}", e)),
                        Err(e) => format!("[error] {}", e),
                    },
                    Err(e) => format!("[error] failed to load '{}': {}", model_name, e),
                }
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

fn make_store_embedding_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "store_embedding",
        [extism::PTR, extism::PTR, extism::PTR],  // key, vector_json, metadata
        [extism::PTR],                             // returns "ok" or error
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let key: String = plugin.memory_get_val(&inputs[0])?;
            let vector_json: String = plugin.memory_get_val(&inputs[1])?;
            let metadata: String = plugin.memory_get_val(&inputs[2])?;

            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = match serde_json::from_str::<Vec<f32>>(&vector_json) {
                Ok(vector) => match storage.store_embedding(&key, &vector, &metadata) {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("[error] {}", e),
                },
                Err(e) => format!("[error] invalid vector JSON: {}", e),
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

fn make_search_by_vector_function(storage: Arc<Storage>) -> Function {
    Function::new(
        "search_by_vector",
        [extism::PTR, extism::PTR],  // query_vector_json, limit
        [extism::PTR],               // returns JSON array of {key, metadata, distance}
        UserData::new(storage),
        |plugin, inputs, outputs, user_data| {
            let query_json: String = plugin.memory_get_val(&inputs[0])?;
            let limit_str: String = plugin.memory_get_val(&inputs[1])?;

            let storage = user_data.get().unwrap();
            let storage = storage.lock().unwrap();

            let response = match serde_json::from_str::<Vec<f32>>(&query_json) {
                Ok(query_vector) => {
                    let limit = limit_str.parse::<u32>().unwrap_or(10);
                    match storage.search_by_vector(&query_vector, limit) {
                        Ok(results) => {
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
                                .unwrap_or_else(|e| format!("[error] {}", e))
                        }
                        Err(e) => format!("[error] {}", e),
                    }
                }
                Err(e) => format!("[error] invalid vector JSON: {}", e),
            };

            let handle = plugin.memory_new(&response)?;
            outputs[0] = extism::Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

