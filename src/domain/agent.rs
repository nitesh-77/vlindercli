use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::agent_manifest::{AgentManifest, MountConfig, ParseError, PromptsConfig, RequirementsConfig};
use super::path::AbsolutePath;
use super::resource_id::ResourceId;
use super::runtime::RuntimeType;

/// An agent with resolved paths, ready for execution.
///
/// All paths (executable, mounts, model URIs) are resolved to absolute paths at load time.
/// See ADR 020 for the manifest format, ADR 048 for identity model.
#[derive(Clone, Debug, Serialize)]
pub struct Agent {
    pub name: String,
    pub description: String,
    pub source: Option<String>,
    pub requirements: Requirements,
    pub prompts: Option<Prompts>,
    pub mounts: Vec<Mount>,
    /// Registry-assigned identity: `<registry_id>/agents/<name>`.
    /// Set by the registry during registration.
    pub id: ResourceId,
    /// Which runtime executes this agent.
    pub runtime: RuntimeType,
    /// Executable reference — native to the runtime ecosystem.
    /// Container: OCI image ref (e.g., "localhost/echo-container:latest")
    /// File: absolute path or file:// URI
    pub executable: String,
    /// Content-addressed image digest resolved at registration time (ADR 073).
    /// For container agents: `podman image inspect` → `sha256:...`
    /// None for non-container runtimes or if resolution failed.
    pub image_digest: Option<String>,
    /// Object storage configuration (optional).
    pub object_storage: Option<ResourceId>,
    /// Vector storage configuration (optional).
    pub vector_storage: Option<ResourceId>,
}

impl Agent {
    /// Create a placeholder ID for agents not yet registered.
    ///
    /// Registry replaces this with `<registry_id>/agents/<name>` during registration.
    pub fn placeholder_id(name: &str) -> ResourceId {
        ResourceId::new(format!("pending-registration://agents/{}", name))
    }

    /// Create an agent directly from TOML content.
    ///
    /// The TOML should contain resolved absolute URIs for `executable` and model paths.
    /// No path resolution is performed - caller is responsible for pre-resolving.
    pub fn from_toml(toml_content: &str) -> Result<Agent, LoadError> {
        let manifest: AgentManifest = toml::from_str(toml_content)
            .map_err(|e| LoadError::Parse(e.to_string()))?;
        Self::from_manifest(manifest)
    }

    /// Create an agent from a manifest.
    ///
    /// All paths in the manifest are already resolved to absolute paths.
    /// The `id` field is set to a placeholder. The registry assigns the real
    /// id (`<registry_id>/agents/<name>`) during registration.
    pub fn from_manifest(manifest: AgentManifest) -> Result<Agent, LoadError> {
        // Validate mounts exist
        let mut mounts = Vec::new();
        for mount_config in manifest.mounts {
            mounts.push(Mount::from_config(mount_config)?);
        }

        let runtime = RuntimeType::from_str(&manifest.runtime)
            .ok_or_else(|| LoadError::Parse(format!("unknown runtime: {}", manifest.runtime)))?;

        Ok(Agent {
            name: manifest.name.clone(),
            description: manifest.description,
            source: manifest.source,
            requirements: Requirements::from_config(manifest.requirements),
            prompts: manifest.prompts.map(|p| p.into()),
            mounts,
            id: Self::placeholder_id(&manifest.name),
            runtime,
            executable: manifest.executable,
            image_digest: None,
            object_storage: manifest.object_storage,
            vector_storage: manifest.vector_storage,
        })
    }

    /// Convenience: load agent from a directory path.
    ///
    /// Looks for `agent.toml` manifest in the directory (ADR 020).
    pub fn load(path: &Path) -> Result<Agent, LoadError> {
        let manifest_path = path.join("agent.toml");

        if !manifest_path.exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("manifest not found: {}", manifest_path.display()),
            )));
        }

        let manifest = AgentManifest::load(&manifest_path)?;
        Self::from_manifest(manifest)
    }

    /// Check if this agent declares a model by name.
    pub fn has_model(&self, model_name: &str) -> bool {
        self.requirements.models.contains_key(model_name)
    }

    /// Get the URI for a model by name.
    pub fn model_uri(&self, model_name: &str) -> Option<&ResourceId> {
        self.requirements.models.get(model_name)
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
            ParseError::ExecutableNotFound(s) => LoadError::Parse(s),
        }
    }
}

/// Agent runtime requirements (validated)
#[derive(Clone, Debug, Serialize)]
pub struct Requirements {
    /// Model name → ResourceId mapping
    pub models: HashMap<String, ResourceId>,
    pub services: Vec<String>,
}

impl Requirements {
    /// Create Requirements from config.
    /// All model URIs are already resolved to absolute by AgentManifest::load().
    fn from_config(config: RequirementsConfig) -> Self {
        let models = config.models
            .into_iter()
            .map(|(name, uri)| (name, ResourceId::new(uri)))
            .collect();

        Requirements {
            models,
            services: config.services,
        }
    }
}

/// Prompt overrides (validated)
#[derive(Clone, Debug, Serialize)]
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
#[derive(Clone, Debug, Serialize)]
pub struct Mount {
    pub host_path: AbsolutePath,
    pub guest_path: PathBuf,
    pub readonly: bool,
}

impl Mount {
    /// Create a Mount from config. Host path is already resolved to absolute.
    fn from_config(config: MountConfig) -> Result<Mount, LoadError> {
        let host_path = AbsolutePath::from_absolute(Path::new(&config.host_path))
            .expect("AgentManifest::load() must produce absolute paths");

        if !host_path.exists() {
            return Err(LoadError::MountNotFound(format!(
                "mount path does not exist: {}",
                host_path
            )));
        }

        Ok(Mount {
            host_path,
            guest_path: PathBuf::from(&config.guest_path),
            readonly: config.mode == "ro",
        })
    }
}
