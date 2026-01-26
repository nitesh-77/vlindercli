use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
    Validation(String),
    PathNotFound(String),
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
