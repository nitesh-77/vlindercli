use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Raw fleet manifest as read from fleet.toml
#[derive(Deserialize)]
pub struct FleetManifest {
    pub name: String,
    pub entry: String,
    pub agents: HashMap<String, AgentEntry>,
}

impl FleetManifest {
    pub fn load(path: &Path) -> Result<FleetManifest, ParseError> {
        let content = std::fs::read_to_string(path)?;
        let manifest: FleetManifest = toml::from_str(&content)?;

        // Internal consistency: entry must reference an agent
        if !manifest.agents.contains_key(&manifest.entry) {
            return Err(ParseError::Validation(format!(
                "entry '{}' not found in agents",
                manifest.entry
            )));
        }

        Ok(manifest)
    }
}

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Toml(String),
    Validation(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "{e}"),
            ParseError::Toml(e) | ParseError::Validation(e) => write!(f, "{e}"),
        }
    }
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

/// Agent entry as declared in fleet.toml
#[derive(Deserialize)]
pub struct AgentEntry {
    pub path: String,
}
