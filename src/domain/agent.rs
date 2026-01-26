use extism::{Function, Manifest, Plugin, Wasm};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Agent runtime requirements
#[derive(Clone, Debug, Deserialize)]
pub struct Requirements {
    pub models: Vec<String>,
    pub services: Vec<String>,
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

/// Filesystem mount declaration for WASI access
#[derive(Clone, Debug, Deserialize)]
pub struct Mount {
    pub host_path: String,
    pub guest_path: String,
    #[serde(default = "default_mount_mode")]
    pub mode: String,
}

fn default_mount_mode() -> String {
    "rw".to_string()
}

/// An agent is its manifest, deserialized
#[derive(Clone, Debug, Deserialize)]
pub struct Agent {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub source: Option<String>,
    /// Path to the WASM binary (required - an agent must have code to run)
    pub code: String,
    pub requirements: Requirements,
    #[serde(default)]
    pub prompts: Option<Prompts>,
    #[serde(default)]
    pub mounts: Vec<Mount>,

    /// Agent directory (set at load time)
    #[serde(skip)]
    pub agent_dir: PathBuf,

    /// Resolved path to WASM binary
    #[serde(skip)]
    wasm_path: PathBuf,

    /// Raw manifest content (for passing to plugin)
    #[serde(skip)]
    manifest_raw: String,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
    Plugin(String),
    MountNotFound(String),
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
    /// Load an agent from a directory path.
    ///
    /// Looks for `agent.toml` manifest and `agent.wasm` binary in the directory (ADR 020).
    /// Mounts are validated at load time (ADR 019).
    pub fn load(path: &Path) -> Result<Agent, LoadError> {
        let agent_dir = path.to_path_buf();
        let manifest_path = agent_dir.join("agent.toml");

        if !manifest_path.exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("manifest not found: {}", manifest_path.display()),
            )));
        }

        let manifest_raw = std::fs::read_to_string(&manifest_path)?;
        let mut agent: Agent = toml::from_str(&manifest_raw)?;

        // Resolve code path (relative paths resolved against agent dir)
        let wasm_path = if Path::new(&agent.code).is_absolute() {
            PathBuf::from(&agent.code)
        } else {
            agent_dir.join(&agent.code)
        };

        if !wasm_path.exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("wasm not found: {}", wasm_path.display()),
            )));
        }

        agent.agent_dir = agent_dir;
        agent.wasm_path = wasm_path;
        agent.manifest_raw = manifest_raw;

        // Validate mounts at load time - fail fast if paths don't exist (ADR 019)
        for mount in &agent.mounts {
            let host_path = if Path::new(&mount.host_path).is_absolute() {
                PathBuf::from(&mount.host_path)
            } else {
                agent.agent_dir.join(&mount.host_path)
            };

            if !host_path.exists() {
                return Err(LoadError::MountNotFound(format!(
                    "mount path does not exist: {}",
                    host_path.display()
                )));
            }
        }

        Ok(agent)
    }

    /// Get the path to the agent's database file
    pub fn db_path(&self) -> PathBuf {
        self.agent_dir.join("agent.db")
    }

    pub fn has_model(&self, name: &str) -> bool {
        self.requirements.models.iter().any(|m| m == name)
    }

    pub fn manifest(&self) -> &str {
        &self.manifest_raw
    }

    /// Resolve mounts to Extism allowed_paths format.
    /// Returns Vec<(host_path_with_ro_prefix, guest_path)>.
    /// No mounts declared → no filesystem access (ADR 019).
    pub fn resolve_mounts(&self) -> Vec<(String, PathBuf)> {
        self.mounts
            .iter()
            .map(|m| {
                // Resolve host path: relative paths against agent dir, absolute as-is
                let host_path = if Path::new(&m.host_path).is_absolute() {
                    PathBuf::from(&m.host_path)
                } else {
                    self.agent_dir.join(&m.host_path)
                };

                // Build key with ro: prefix if read-only
                let key = if m.mode == "ro" {
                    format!("ro:{}", host_path.display())
                } else {
                    host_path.display().to_string()
                };

                (key, PathBuf::from(&m.guest_path))
            })
            .collect()
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

        // Build manifest with WASI allowed paths from mounts
        let mut manifest = Manifest::new([wasm]).with_allowed_host("*");

        for (host_path, guest_path) in self.resolve_mounts() {
            manifest = manifest.with_allowed_path(host_path, guest_path);
        }

        // Plugin::new third param (true) enables WASI
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
