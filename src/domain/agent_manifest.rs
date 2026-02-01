use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use super::resource_id::ResourceId;

/// Agent manifest as read from agent.toml.
///
/// The `id` field is resolved to a URI during loading. Relative paths
/// are resolved against the manifest's directory.
#[derive(Clone, Debug, Deserialize)]
pub struct AgentManifest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub source: Option<String>,
    pub id: String,
    pub requirements: RequirementsConfig,
    #[serde(default)]
    pub prompts: Option<PromptsConfig>,
    #[serde(default)]
    pub mounts: Vec<MountConfig>,
    /// Object storage resource ID (e.g., "sqlite:///path/to/objects.db")
    #[serde(default)]
    pub object_storage: Option<ResourceId>,
    /// Vector storage resource ID (e.g., "sqlite:///path/to/vectors.db")
    #[serde(default)]
    pub vector_storage: Option<ResourceId>,
}

impl AgentManifest {
    /// Load an agent manifest from a file path.
    ///
    /// Resolves all paths to absolute URIs:
    /// - `id` → file:// URI (for local files)
    /// - `requirements.models` values → file:// URIs
    /// - `mounts[].host_path` → absolute paths
    pub fn load(path: &Path) -> Result<AgentManifest, ParseError> {
        let content = std::fs::read_to_string(path)?;
        let mut manifest: AgentManifest = toml::from_str(&content)?;

        // Derive agent directory from manifest path, ensuring it's absolute
        let agent_dir = path
            .parent()
            .unwrap_or(Path::new("."))
            .canonicalize()
            .unwrap_or_else(|_| path.parent().unwrap_or(Path::new(".")).to_path_buf());

        // Resolve id to URI
        manifest.id = resolve_id_uri(&manifest.id, &agent_dir)?;

        // Resolve model URIs
        manifest.requirements.models = manifest.requirements.models
            .into_iter()
            .map(|(name, uri)| (name, resolve_uri(&uri, &agent_dir)))
            .collect();

        // Resolve mount host paths to absolute
        for mount in &mut manifest.mounts {
            mount.host_path = resolve_host_path(&mount.host_path, &agent_dir);
        }

        Ok(manifest)
    }
}

/// Resolve an id reference to a URI.
///
/// - If already a URI (contains "://"), return as-is (but resolve relative file:// paths)
/// - If absolute path, convert to file:// URI
/// - If relative path, resolve against agent_dir and convert to file:// URI
fn resolve_id_uri(id: &str, agent_dir: &Path) -> Result<String, ParseError> {
    // Already a URI
    if id.contains("://") {
        // For file:// URIs with relative paths, resolve them
        if let Some(path) = id.strip_prefix("file://") {
            if !Path::new(path).is_absolute() {
                let resolved = agent_dir.join(path);
                if !resolved.exists() {
                    return Err(ParseError::IdNotFound(format!(
                        "id not found: {}",
                        resolved.display()
                    )));
                }
                return Ok(format!("file://{}", resolved.display()));
            }
        }
        return Ok(id.to_string());
    }

    // Resolve path (relative or absolute)
    let id_path = if Path::new(id).is_absolute() {
        Path::new(id).to_path_buf()
    } else {
        agent_dir.join(id)
    };

    if !id_path.exists() {
        return Err(ParseError::IdNotFound(format!(
            "id not found: {}",
            id_path.display()
        )));
    }

    Ok(format!("file://{}", id_path.display()))
}

/// Resolve a URI, making relative file:// URIs absolute.
///
/// - If already absolute or non-file URI, return as-is
/// - If relative file:// URI, resolve against agent_dir
fn resolve_uri(uri: &str, agent_dir: &Path) -> String {
    if let Some(path) = uri.strip_prefix("file://") {
        if path.starts_with("./") || !Path::new(path).is_absolute() {
            let clean_path = path.strip_prefix("./").unwrap_or(path);
            let resolved = agent_dir.join(clean_path);
            return format!("file://{}", resolved.display());
        }
    }
    uri.to_string()
}

/// Resolve a host path, making relative paths absolute.
fn resolve_host_path(host_path: &str, agent_dir: &Path) -> String {
    if Path::new(host_path).is_absolute() {
        host_path.to_string()
    } else {
        agent_dir.join(host_path).display().to_string()
    }
}

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Toml(String),
    IdNotFound(String),
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

/// Requirements as declared in agent.toml
#[derive(Clone, Debug, Deserialize)]
pub struct RequirementsConfig {
    /// Model name → URI mapping (e.g., "phi3" = "file://./models/phi3.toml")
    #[serde(default)]
    pub models: HashMap<String, String>,
    pub services: Vec<String>,
}

/// Prompt overrides as declared in agent.toml
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PromptsConfig {
    pub intent_recognition: Option<String>,
    pub query_expansion: Option<String>,
    pub answer_generation: Option<String>,
    pub map_summarize: Option<String>,
    pub reduce_summaries: Option<String>,
    pub direct_summarize: Option<String>,
}

/// Mount declaration as declared in agent.toml
#[derive(Clone, Debug, Deserialize)]
pub struct MountConfig {
    pub host_path: String,
    pub guest_path: String,
    #[serde(default = "default_mount_mode")]
    pub mode: String,
}

fn default_mount_mode() -> String {
    "rw".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_with_storage() {
        let toml = r#"
            name = "test-agent"
            description = "Test agent with storage"
            id = "agent.wasm"
            object_storage = "sqlite:///data/objects.db"
            vector_storage = "sqlite:///data/vectors.db"

            [requirements]
            services = ["inference"]
        "#;

        let manifest: AgentManifest = toml::from_str(toml).unwrap();

        assert_eq!(manifest.name, "test-agent");
        assert_eq!(
            manifest.object_storage.as_ref().map(|r| r.as_str()),
            Some("sqlite:///data/objects.db")
        );
        assert_eq!(
            manifest.vector_storage.as_ref().map(|r| r.as_str()),
            Some("sqlite:///data/vectors.db")
        );
    }

    #[test]
    fn parse_manifest_without_storage() {
        let toml = r#"
            name = "test-agent"
            description = "Test agent without storage"
            id = "agent.wasm"

            [requirements]
            services = ["inference"]
        "#;

        let manifest: AgentManifest = toml::from_str(toml).unwrap();

        assert_eq!(manifest.name, "test-agent");
        assert!(manifest.object_storage.is_none());
        assert!(manifest.vector_storage.is_none());
    }

    #[test]
    fn parse_manifest_with_partial_storage() {
        let toml = r#"
            name = "test-agent"
            description = "Test agent with only object storage"
            id = "agent.wasm"
            object_storage = "s3://my-bucket/agents/test"

            [requirements]
            services = []
        "#;

        let manifest: AgentManifest = toml::from_str(toml).unwrap();

        assert_eq!(
            manifest.object_storage.as_ref().map(|r| r.as_str()),
            Some("s3://my-bucket/agents/test")
        );
        assert!(manifest.vector_storage.is_none());
    }

    #[test]
    fn storage_resource_id_scheme() {
        let toml = r#"
            name = "test-agent"
            description = "Test"
            id = "agent.wasm"
            object_storage = "memory://test"
            vector_storage = "pinecone://my-index"

            [requirements]
            services = []
        "#;

        let manifest: AgentManifest = toml::from_str(toml).unwrap();

        assert_eq!(
            manifest.object_storage.as_ref().and_then(|r| r.scheme()),
            Some("memory")
        );
        assert_eq!(
            manifest.vector_storage.as_ref().and_then(|r| r.scheme()),
            Some("pinecone")
        );
    }
}
