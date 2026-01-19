use crate::config;
use extism::{Function, Manifest, Plugin, Wasm};
use serde::Deserialize;

/// Agent runtime requirements
#[derive(Clone, Debug, Deserialize)]
pub struct Requirements {
    pub models: Vec<String>,
    pub host_capabilities: Vec<String>,
}

/// Optional prompt overrides
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Prompts {
    pub intent_recognition: Option<String>,
    pub query_expansion: Option<String>,
    pub answer_generation: Option<String>,
    pub map_summarize: Option<String>,
    pub reduce_summaries: Option<String>,
    pub direct_summarize: Option<String>,
}

/// An agent is a Vlinderfile, deserialized
#[derive(Clone, Debug, Deserialize)]
pub struct Agent {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub source: Option<String>,
    pub requirements: Requirements,
    #[serde(default)]
    pub prompts: Option<Prompts>,

    /// Path to WASM binary (derived, not in Vlinderfile)
    #[serde(skip)]
    wasm_path: String,

    /// Raw Vlinderfile content (for passing to plugin)
    #[serde(skip)]
    vlinderfile_raw: String,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
    Plugin(String),
}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl From<toml::de::Error> for LoadError {
    fn from(e: toml::de::Error) -> Self {
        LoadError::Parse(e.to_string())
    }
}

impl From<extism::Error> for LoadError {
    fn from(e: extism::Error) -> Self {
        LoadError::Plugin(e.to_string())
    }
}

impl Agent {
    pub fn load(name: &str) -> Result<Agent, LoadError> {
        let vlinderfile_path = config::agent_vlinderfile_path(name);
        let wasm_path = config::agent_wasm_path(name);

        if !vlinderfile_path.exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Vlinderfile not found: {}", vlinderfile_path.display()),
            )));
        }

        if !wasm_path.exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("wasm not found: {}", wasm_path.display()),
            )));
        }

        let vlinderfile_raw = std::fs::read_to_string(&vlinderfile_path)?;
        let mut agent: Agent = toml::from_str(&vlinderfile_raw)?;

        agent.wasm_path = wasm_path.to_string_lossy().to_string();
        agent.vlinderfile_raw = vlinderfile_raw;

        Ok(agent)
    }

    pub fn has_model(&self, name: &str) -> bool {
        self.requirements.models.iter().any(|m| m == name)
    }

    pub fn vlinderfile(&self) -> &str {
        &self.vlinderfile_raw
    }

    pub fn execute(&self, input: &str) -> String {
        self.execute_with_functions(input, [])
    }

    pub fn execute_with_functions(
        &self,
        input: &str,
        functions: impl IntoIterator<Item = Function>,
    ) -> String {
        let wasm = Wasm::file(&self.wasm_path);
        let manifest = Manifest::new([wasm]).with_allowed_host("*");
        let mut plugin = Plugin::new(&manifest, functions, true).unwrap();
        plugin.call("process", input).unwrap()
    }
}
