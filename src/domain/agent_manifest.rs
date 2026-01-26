use serde::Deserialize;
use std::path::Path;

/// Raw agent manifest as read from agent.toml
#[derive(Clone, Debug, Deserialize)]
pub struct AgentManifest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub source: Option<String>,
    pub code: String,
    pub requirements: RequirementsConfig,
    #[serde(default)]
    pub prompts: Option<PromptsConfig>,
    #[serde(default)]
    pub mounts: Vec<MountConfig>,
}

impl AgentManifest {
    /// Load an agent manifest from a file path.
    pub fn load(path: &Path) -> Result<(AgentManifest, String), ParseError> {
        let content = std::fs::read_to_string(path)?;
        let manifest: AgentManifest = toml::from_str(&content)?;
        Ok((manifest, content))
    }
}

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Toml(String),
}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        ParseError::Io(e)
    }
}

impl From<toml::de::Error> for ParseError {
    fn from(e: toml::de::Error) -> Self {
        ParseError::Toml(e.to_string())
    }
}

/// Requirements as declared in agent.toml
#[derive(Clone, Debug, Deserialize)]
pub struct RequirementsConfig {
    pub models: Vec<String>,
    pub services: Vec<String>,
}

/// Prompt overrides as declared in agent.toml
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PromptsConfig {
    pub intent_recognition: Option<String>,
    pub query_expansion: Option<String>,
    pub answer_generation: Option<String>,
    pub map_summarize: Option<String>,
    pub reduce_summaries: Option<String>,
    pub direct_summarize: Option<String>,
}

/// Mount declaration as declared in agent.toml
#[derive(Clone, Debug, Deserialize)]
pub struct MountConfig {
    pub host_path: String,
    pub guest_path: String,
    #[serde(default = "default_mount_mode")]
    pub mode: String,
}

fn default_mount_mode() -> String {
    "rw".to_string()
}
