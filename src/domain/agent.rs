use std::path::{Path, PathBuf};

use super::agent_manifest::{AgentManifest, MountConfig, ParseError, PromptsConfig, RequirementsConfig};

/// An agent with resolved paths, ready for execution.
///
/// See ADR 020 for the manifest format.
#[derive(Clone, Debug)]
pub struct Agent {
    pub name: String,
    pub description: String,
    pub source: Option<String>,
    pub requirements: Requirements,
    pub prompts: Option<Prompts>,
    pub mounts: Vec<Mount>,
    pub agent_dir: PathBuf,
    /// URI pointing to the agent's executable code (e.g., "file:///path/to/agent.wasm")
    pub code: String,
    manifest_raw: String,
}

impl Agent {
    /// Create an agent from a manifest, resolving paths against agent_dir.
    ///
    /// Validates environmental constraints (code exists, mounts exist).
    pub fn from_manifest(
        manifest: AgentManifest,
        manifest_raw: String,
        agent_dir: &Path,
    ) -> Result<Agent, LoadError> {
        let agent_dir = agent_dir.to_path_buf();

        // Resolve code path to absolute, then convert to URI
        let code_path = if Path::new(&manifest.code).is_absolute() {
            PathBuf::from(&manifest.code)
        } else {
            agent_dir.join(&manifest.code)
        };

        if !code_path.exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("code not found: {}", code_path.display()),
            )));
        }

        let code = format!("file://{}", code_path.display());

        // Resolve and validate mounts
        let mut mounts = Vec::new();
        for mount_config in manifest.mounts {
            mounts.push(Mount::from_config(mount_config, &agent_dir)?);
        }

        Ok(Agent {
            name: manifest.name,
            description: manifest.description,
            source: manifest.source,
            requirements: manifest.requirements.into(),
            prompts: manifest.prompts.map(|p| p.into()),
            mounts,
            agent_dir,
            code,
            manifest_raw,
        })
    }

    /// Convenience: load agent from a directory path.
    ///
    /// Looks for `agent.toml` manifest in the directory (ADR 020).
    pub fn load(path: &Path) -> Result<Agent, LoadError> {
        let agent_dir = path.to_path_buf();
        let manifest_path = agent_dir.join("agent.toml");

        if !manifest_path.exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("manifest not found: {}", manifest_path.display()),
            )));
        }

        let (manifest, raw) = AgentManifest::load(&manifest_path)?;
        Self::from_manifest(manifest, raw, &agent_dir)
    }

    pub fn db_path(&self) -> PathBuf {
        self.agent_dir.join("agent.db")
    }

    pub fn has_model(&self, name: &str) -> bool {
        self.requirements.models.iter().any(|m| m == name)
    }

    pub fn manifest(&self) -> &str {
        &self.manifest_raw
    }
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
    MountNotFound(String),
}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl From<ParseError> for LoadError {
    fn from(e: ParseError) -> Self {
        match e {
            ParseError::Io(e) => LoadError::Io(e),
            ParseError::Toml(s) => LoadError::Parse(s),
        }
    }
}

/// Agent runtime requirements (validated)
#[derive(Clone, Debug)]
pub struct Requirements {
    pub models: Vec<String>,
    pub services: Vec<String>,
}

impl From<RequirementsConfig> for Requirements {
    fn from(config: RequirementsConfig) -> Self {
        Requirements {
            models: config.models,
            services: config.services,
        }
    }
}

/// Prompt overrides (validated)
#[derive(Clone, Debug)]
pub struct Prompts {
    pub intent_recognition: Option<String>,
    pub query_expansion: Option<String>,
    pub answer_generation: Option<String>,
    pub map_summarize: Option<String>,
    pub reduce_summaries: Option<String>,
    pub direct_summarize: Option<String>,
}

impl From<PromptsConfig> for Prompts {
    fn from(config: PromptsConfig) -> Self {
        Prompts {
            intent_recognition: config.intent_recognition,
            query_expansion: config.query_expansion,
            answer_generation: config.answer_generation,
            map_summarize: config.map_summarize,
            reduce_summaries: config.reduce_summaries,
            direct_summarize: config.direct_summarize,
        }
    }
}

/// Resolved filesystem mount for WASI access
#[derive(Clone, Debug)]
pub struct Mount {
    pub host_path: PathBuf,
    pub guest_path: PathBuf,
    pub readonly: bool,
}

impl Mount {
    fn from_config(config: MountConfig, agent_dir: &Path) -> Result<Mount, LoadError> {
        let host_path = if Path::new(&config.host_path).is_absolute() {
            PathBuf::from(&config.host_path)
        } else {
            agent_dir.join(&config.host_path)
        };

        if !host_path.exists() {
            return Err(LoadError::MountNotFound(format!(
                "mount path does not exist: {}",
                host_path.display()
            )));
        }

        Ok(Mount {
            host_path,
            guest_path: PathBuf::from(&config.guest_path),
            readonly: config.mode == "ro",
        })
    }
}
