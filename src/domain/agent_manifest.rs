use serde::Deserialize;
use std::path::Path;

/// Agent manifest as read from agent.toml.
///
/// The `code` field is resolved to a URI during loading. Relative paths
/// are resolved against the manifest's directory.
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
    ///
    /// Resolves `code` to a URI and validates the file exists.
    pub fn load(path: &Path) -> Result<(AgentManifest, String), ParseError> {
        let content = std::fs::read_to_string(path)?;
        let mut manifest: AgentManifest = toml::from_str(&content)?;

        // Derive agent directory from manifest path
        let agent_dir = path.parent().unwrap_or(Path::new("."));

        // Resolve code to URI
        manifest.code = resolve_code_uri(&manifest.code, agent_dir)?;

        Ok((manifest, content))
    }
}

/// Resolve a code reference to a URI.
///
/// - If already a URI (contains "://"), return as-is
/// - If absolute path, convert to file:// URI
/// - If relative path, resolve against agent_dir and convert to file:// URI
fn resolve_code_uri(code: &str, agent_dir: &Path) -> Result<String, ParseError> {
    // Already a URI
    if code.contains("://") {
        return Ok(code.to_string());
    }

    // Resolve path (relative or absolute)
    let code_path = if Path::new(code).is_absolute() {
        Path::new(code).to_path_buf()
    } else {
        agent_dir.join(code)
    };

    if !code_path.exists() {
        return Err(ParseError::CodeNotFound(format!(
            "code not found: {}",
            code_path.display()
        )));
    }

    Ok(format!("file://{}", code_path.display()))
}

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Toml(String),
    CodeNotFound(String),
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
