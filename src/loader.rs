use std::path::Path;

use crate::domain::{Agent, AgentLoadError, Fleet, FleetLoadError, Model, ModelLoadError};

// ============================================================================
// Public API (free functions that dispatch by URI scheme)
// ============================================================================

pub fn load_agent(uri: &str) -> Result<Agent, LoadError> {
    match parse_scheme(uri) {
        "file" => FileLoader.load_agent(uri),
        scheme => Err(LoadError::NotFound(format!("unknown scheme: {}", scheme))),
    }
}

pub fn load_fleet(uri: &str) -> Result<Fleet, LoadError> {
    match parse_scheme(uri) {
        "file" => FileLoader.load_fleet(uri),
        scheme => Err(LoadError::NotFound(format!("unknown scheme: {}", scheme))),
    }
}

pub fn load_model(uri: &str) -> Result<Model, LoadError> {
    match parse_scheme(uri) {
        "file" => FileLoader.load_model(uri),
        scheme => Err(LoadError::NotFound(format!("unknown scheme: {}", scheme))),
    }
}

fn parse_scheme(uri: &str) -> &str {
    uri.split("://").next().unwrap_or("file")
}

// ============================================================================
// Trait and Implementations
// ============================================================================

/// Loads agents, fleets, and models from URIs.
pub trait Loader {
    fn load_agent(&self, uri: &str) -> Result<Agent, LoadError>;
    fn load_fleet(&self, uri: &str) -> Result<Fleet, LoadError>;
    fn load_model(&self, uri: &str) -> Result<Model, LoadError>;
}

/// Loads agents and fleets from the filesystem (file:// scheme).
struct FileLoader;

impl FileLoader {
    fn uri_to_path(uri: &str) -> &str {
        uri.strip_prefix("file://").unwrap_or(uri)
    }
}

impl Loader for FileLoader {
    fn load_agent(&self, uri: &str) -> Result<Agent, LoadError> {
        let path = Path::new(Self::uri_to_path(uri));
        Agent::load(path).map_err(LoadError::from)
    }

    fn load_fleet(&self, uri: &str) -> Result<Fleet, LoadError> {
        let path = Path::new(Self::uri_to_path(uri));
        Fleet::load(path).map_err(LoadError::from)
    }

    fn load_model(&self, uri: &str) -> Result<Model, LoadError> {
        let path = Path::new(Self::uri_to_path(uri));
        Model::load(path).map_err(LoadError::from)
    }
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
            AgentLoadError::MountNotFound(s) => LoadError::Validation(s),
        }
    }
}

impl From<FleetLoadError> for LoadError {
    fn from(e: FleetLoadError) -> Self {
        match e {
            FleetLoadError::Io(e) => LoadError::Io(e),
            FleetLoadError::Parse(s) => LoadError::Parse(s),
            FleetLoadError::Validation(s) => LoadError::Validation(s),
            FleetLoadError::PathNotFound(s) => LoadError::NotFound(s),
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_uri(name: &str) -> String {
        let path = PathBuf::from("tests/fixtures/agents")
            .join(name)
            .canonicalize()
            .unwrap();
        format!("file://{}", path.display())
    }

    fn fleet_fixture_uri(name: &str) -> String {
        let path = PathBuf::from("tests/fixtures/fleets")
            .join(name)
            .canonicalize()
            .unwrap();
        format!("file://{}", path.display())
    }

    #[test]
    fn uri_to_path_strips_file_prefix() {
        assert_eq!(
            FileLoader::uri_to_path("file:///home/user/agent"),
            "/home/user/agent"
        );
    }

    #[test]
    fn uri_to_path_handles_missing_prefix() {
        assert_eq!(
            FileLoader::uri_to_path("/home/user/agent"),
            "/home/user/agent"
        );
    }

    #[test]
    fn load_agent_with_file_uri() {
        let agent = load_agent(&fixture_uri("echo-agent")).unwrap();
        assert_eq!(agent.name, "echo-agent");
    }

    #[test]
    fn load_agent_fails_for_missing_uri() {
        let result = load_agent("file:///nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn load_fleet_with_file_uri() {
        let fleet = load_fleet(&fleet_fixture_uri("test-fleet")).unwrap();
        assert_eq!(fleet.name, "test-fleet");
    }

    #[test]
    fn load_fleet_fails_for_missing_uri() {
        let result = load_fleet("file:///nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn load_agent_rejects_unknown_scheme() {
        let result = load_agent("registry://some-agent");
        assert!(result.is_err());
    }

    // ========================================================================
    // load_model tests
    // ========================================================================

    fn model_fixture_uri(agent: &str, model_file: &str) -> String {
        let path = PathBuf::from("tests/fixtures/agents")
            .join(agent)
            .join("models")
            .join(model_file)
            .canonicalize()
            .unwrap();
        format!("file://{}", path.display())
    }

    #[test]
    fn load_model_with_file_uri() {
        let model = load_model(&model_fixture_uri("model-test-agent", "inference.toml")).unwrap();
        assert_eq!(model.name, "phi3");
    }

    #[test]
    fn load_model_fails_for_missing_uri() {
        let result = load_model("file:///nonexistent/model.toml");
        assert!(result.is_err());
    }

    #[test]
    fn load_model_rejects_unknown_scheme() {
        let result = load_model("http://example.com/model.toml");
        assert!(result.is_err());

        let result = load_model("ollama://phi3");
        assert!(result.is_err());
    }

    #[test]
    fn load_model_returns_correct_type() {
        let inference = load_model(&model_fixture_uri("model-test-agent", "inference.toml")).unwrap();
        assert_eq!(inference.model_type, crate::domain::ModelType::Inference);

        let embedding = load_model(&model_fixture_uri("model-test-agent", "embedding.toml")).unwrap();
        assert_eq!(embedding.model_type, crate::domain::ModelType::Embedding);
    }
}
