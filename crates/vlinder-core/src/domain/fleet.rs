use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::agent::Agent;
use super::fleet_manifest::{FleetManifest, ParseError};

/// A fleet is a composition boundary for agents.
///
/// See ADR 022 for the manifest format.
#[derive(Clone, Debug)]
pub struct Fleet {
    pub name: String,
    pub entry: String,
    pub project_dir: PathBuf,
    agents: HashMap<String, PathBuf>,
}

impl Fleet {
    pub fn from_manifest(manifest: FleetManifest, project_dir: &Path) -> Result<Fleet, LoadError> {
        let project_dir = project_dir.to_path_buf();
        let mut agents = HashMap::new();

        for (name, entry) in manifest.agents {
            let full_path = project_dir.join(&entry.path);
            if !full_path.exists() {
                return Err(LoadError::PathNotFound(format!(
                    "agent '{}' path does not exist: {}",
                    name,
                    full_path.display()
                )));
            }
            agents.insert(name, full_path);
        }

        Ok(Fleet {
            name: manifest.name,
            entry: manifest.entry,
            agents,
            project_dir,
        })
    }

    /// Convenience: load fleet from project directory
    pub fn load(project_dir: &Path) -> Result<Fleet, LoadError> {
        let manifest_path = project_dir.join("fleet.toml");
        let manifest = FleetManifest::load(&manifest_path)?;
        Self::from_manifest(manifest, project_dir)
    }

    pub fn has_agent(&self, name: &str) -> bool {
        self.agents.contains_key(name)
    }

    pub fn agent_path(&self, name: &str) -> Option<&Path> {
        self.agents.get(name).map(|p| p.as_path())
    }

    /// Iterate over all (name, path) pairs.
    pub fn agents(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.agents.iter().map(|(name, path)| (name.as_str(), path.as_path()))
    }

    /// Build a context string describing the fleet for the entry agent.
    ///
    /// Lists all non-entry agents with their descriptions and model info.
    /// The entry agent uses this to know what it can delegate to.
    pub fn build_context(&self) -> Result<String, LoadError> {
        let mut lines = vec![
            format!("Fleet: {}", self.name),
            "Available agents for delegation (use /delegate endpoint):".to_string(),
        ];

        for (name, path) in &self.agents {
            if *name == self.entry {
                continue; // skip entry agent — it's the one reading this context
            }
            match Agent::load(path) {
                Ok(agent) => {
                    let models: Vec<String> = agent.requirements.models.keys().cloned().collect();
                    let model_info = if models.is_empty() {
                        String::new()
                    } else {
                        format!(" Models: {}.", models.join(", "))
                    };
                    lines.push(format!("- {}: {}.{}", name, agent.description, model_info));
                }
                Err(_) => {
                    lines.push(format!("- {}: (could not load agent description)", name));
                }
            }
        }

        Ok(lines.join("\n"))
    }
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
    Validation(String),
    PathNotFound(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "IO error: {}", e),
            LoadError::Parse(s) => write!(f, "parse error: {}", s),
            LoadError::Validation(s) => write!(f, "validation error: {}", s),
            LoadError::PathNotFound(s) => write!(f, "path not found: {}", s),
        }
    }
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
            ParseError::Validation(s) => LoadError::Validation(s),
        }
    }
}
