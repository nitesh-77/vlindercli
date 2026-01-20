use crate::config;
use extism::{Function, Manifest, Plugin, Wasm};
use serde::Deserialize;
use std::path::{Path, PathBuf};

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
    #[serde(default)]
    pub mounts: Vec<Mount>,

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

    /// Resolve mounts to Extism allowed_paths format.
    /// Returns Vec<(host_path_with_ro_prefix, guest_path)>.
    /// If no mounts are declared, returns the default mount: mnt -> /
    pub fn resolve_mounts(&self) -> Vec<(String, PathBuf)> {
        let mounts = if self.mounts.is_empty() {
            // Default mount: agent's mnt dir -> /
            vec![Mount {
                host_path: "mnt".to_string(),
                guest_path: "/".to_string(),
                mode: "rw".to_string(),
            }]
        } else {
            self.mounts.clone()
        };

        mounts
            .iter()
            .filter_map(|m| {
                // Resolve host path: relative paths against agent dir, absolute as-is
                let host_path = if Path::new(&m.host_path).is_absolute() {
                    PathBuf::from(&m.host_path)
                } else {
                    config::agent_dir(&self.name).join(&m.host_path)
                };

                // Skip if host path doesn't exist
                if !host_path.exists() {
                    tracing::warn!("Mount host_path does not exist: {:?}", host_path);
                    return None;
                }

                // Build key with ro: prefix if read-only
                let key = if m.mode == "ro" {
                    format!("ro:{}", host_path.display())
                } else {
                    host_path.display().to_string()
                };

                Some((key, PathBuf::from(&m.guest_path)))
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
