use super::{Agent, AgentLoadError, FleetManifest, FleetManifestParseError, Model, ModelLoadError};

// ============================================================================
// Trait
// ============================================================================

/// Loads agents, fleet manifests, and models from URIs.
pub trait Loader {
    fn load_agent(&self, uri: &str) -> Result<Agent, LoadError>;
    fn load_fleet_manifest(&self, uri: &str) -> Result<FleetManifest, LoadError>;
    fn load_model(&self, uri: &str) -> Result<Model, LoadError>;
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum LoadError {
    NotFound(String),
    Parse(String),
    Validation(String),
    Io(std::io::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotFound(s) => write!(f, "not found: {}", s),
            LoadError::Parse(s) => write!(f, "parse error: {}", s),
            LoadError::Validation(s) => write!(f, "validation error: {}", s),
            LoadError::Io(e) => write!(f, "io error: {}", e),
        }
    }
}

impl From<AgentLoadError> for LoadError {
    fn from(e: AgentLoadError) -> Self {
        match e {
            AgentLoadError::Io(e) => LoadError::Io(e),
            AgentLoadError::Parse(s) => LoadError::Parse(s),
        }
    }
}

impl From<FleetManifestParseError> for LoadError {
    fn from(e: FleetManifestParseError) -> Self {
        match e {
            FleetManifestParseError::Io(e) => LoadError::Io(e),
            FleetManifestParseError::Toml(s) => LoadError::Parse(s),
            FleetManifestParseError::Validation(s) => LoadError::Validation(s),
        }
    }
}

impl From<ModelLoadError> for LoadError {
    fn from(e: ModelLoadError) -> Self {
        match e {
            ModelLoadError::Io(e) => LoadError::Io(e),
            ModelLoadError::Parse(s) => LoadError::Parse(s),
        }
    }
}
