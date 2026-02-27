use std::path::Path;

use vlinder_core::domain::{Agent, FleetManifest, LoadError, Loader, Model};

// ============================================================================
// Public API (free functions that dispatch by URI scheme)
// ============================================================================

pub fn load_agent(uri: &str) -> Result<Agent, LoadError> {
    match parse_scheme(uri) {
        "file" => FileLoader.load_agent(uri),
        scheme => Err(LoadError::NotFound(format!("unknown scheme: {}", scheme))),
    }
}

pub fn load_fleet_manifest(uri: &str) -> Result<FleetManifest, LoadError> {
    match parse_scheme(uri) {
        "file" => FileLoader.load_fleet_manifest(uri),
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
// FileLoader — filesystem implementation
// ============================================================================

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

    fn load_fleet_manifest(&self, uri: &str) -> Result<FleetManifest, LoadError> {
        let path = Path::new(Self::uri_to_path(uri));
        let manifest_path = path.join("fleet.toml");
        FleetManifest::load(&manifest_path).map_err(LoadError::from)
    }

    fn load_model(&self, uri: &str) -> Result<Model, LoadError> {
        let path = Path::new(Self::uri_to_path(uri));
        Model::load(path).map_err(LoadError::from)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
    fn load_agent_fails_for_missing_uri() {
        let result = load_agent("file:///nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn load_fleet_manifest_fails_for_missing_uri() {
        let result = load_fleet_manifest("file:///nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn load_agent_rejects_unknown_scheme() {
        let result = load_agent("registry://some-agent");
        assert!(result.is_err());
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
}
