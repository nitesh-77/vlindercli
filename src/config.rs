use std::path::PathBuf;

/// Base directory for vlinder data (agents, models, storage)
///
/// Resolution order (TODO: implement layers):
/// 1. CLI args (future)
/// 2. Env var VLINDER_DIR (future)
/// 3. Config file (future)
/// 4. Default: .vlinder in current directory
pub fn vlinder_dir() -> PathBuf {
    // TODO: CLI args
    // TODO: std::env::var("VLINDER_DIR")
    // TODO: confy config file
    PathBuf::from(".vlinder")
}

pub fn agents_dir() -> PathBuf {
    vlinder_dir().join("agents")
}

pub fn models_dir() -> PathBuf {
    vlinder_dir().join("models")
}

pub fn agent_dir(name: &str) -> PathBuf {
    agents_dir().join(name)
}

pub fn agent_wasm_path(name: &str) -> PathBuf {
    let wasm_name = name.replace('-', "_");
    agent_dir(name).join(format!("{}.wasm", wasm_name))
}

pub fn agent_vlinderfile_path(name: &str) -> PathBuf {
    agent_dir(name).join("Vlinderfile")
}

pub fn agent_db_path(name: &str) -> PathBuf {
    agent_dir(name).join(format!("{}.db", name))
}

pub fn model_path(model_name: &str) -> PathBuf {
    models_dir().join(format!("{}.gguf", model_name))
}
