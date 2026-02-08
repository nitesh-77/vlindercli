use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use super::resource_id::ResourceId;

/// Agent manifest as read from agent.toml.
///
/// Relative paths (models, mounts, storage) are resolved against
/// the manifest's directory during loading.
#[derive(Clone, Debug, Deserialize)]
pub struct AgentManifest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub source: Option<String>,
    /// Runtime type (e.g., "container"). Determines which executor runs the agent.
    pub runtime: String,
    /// Executable reference — native to the runtime ecosystem.
    /// Container: OCI image ref (e.g., "localhost/echo-container:latest")
    /// File paths are resolved to absolute during loading.
    pub executable: String,
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
    /// Resolves relative paths to absolute:
    /// - `executable` → resolved only for file-based runtimes
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

        // Resolve executable for file-based runtimes only.
        // Container image refs (e.g., "localhost/my-agent:latest") are passed through as-is.
        if manifest.runtime != "container" {
            manifest.executable = resolve_executable_path(&manifest.executable, &agent_dir)?;
        }

        // Resolve model URIs
        manifest.requirements.models = manifest.requirements.models
            .into_iter()
            .map(|(name, uri)| (name, resolve_uri(&uri, &agent_dir)))
            .collect();

        // Resolve mount host paths to absolute
        for mount in &mut manifest.mounts {
            mount.host_path = resolve_host_path(&mount.host_path, &agent_dir);
        }

        // Resolve storage URIs (sqlite:// paths need to be absolute)
        if let Some(ref storage) = manifest.object_storage {
            manifest.object_storage = Some(resolve_storage_uri(storage, &agent_dir));
        }
        if let Some(ref storage) = manifest.vector_storage {
            manifest.vector_storage = Some(resolve_storage_uri(storage, &agent_dir));
        }

        Ok(manifest)
    }
}

/// Resolve a file-based executable path to an absolute file:// URI.
///
/// Only used for non-container runtimes where the executable is a local file.
/// - If already a file:// URI, resolve relative paths
/// - If a bare path, resolve against agent_dir and convert to file:// URI
fn resolve_executable_path(executable: &str, agent_dir: &Path) -> Result<String, ParseError> {
    // Already a file:// URI with relative path
    if let Some(path) = executable.strip_prefix("file://") {
        if !Path::new(path).is_absolute() {
            let resolved = agent_dir.join(path);
            if !resolved.exists() {
                return Err(ParseError::ExecutableNotFound(format!(
                    "executable not found: {}",
                    resolved.display()
                )));
            }
            return Ok(format!("file://{}", resolved.display()));
        }
        return Ok(executable.to_string());
    }

    // Already a URI of another scheme — pass through
    if executable.contains("://") {
        return Ok(executable.to_string());
    }

    // Resolve bare path (relative or absolute)
    let exe_path = if Path::new(executable).is_absolute() {
        Path::new(executable).to_path_buf()
    } else {
        agent_dir.join(executable)
    };

    if !exe_path.exists() {
        return Err(ParseError::ExecutableNotFound(format!(
            "executable not found: {}",
            exe_path.display()
        )));
    }

    Ok(format!("file://{}", exe_path.display()))
}

/// Resolve a URI, making relative file:// URIs absolute.
///
/// - Non-file URIs (http://, etc.) are returned as-is
/// - Absolute file:// URIs are returned as-is
/// - Relative file:// URIs are resolved against agent_dir
fn resolve_uri(uri: &str, agent_dir: &Path) -> String {
    // Non-file URIs pass through unchanged (e.g., http://127.0.0.1:9000/models/phi3)
    if uri.contains("://") && !uri.starts_with("file://") {
        return uri.to_string();
    }

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
///
/// Handles tilde expansion: `~/foo` → `/home/user/foo`.
fn resolve_host_path(host_path: &str, agent_dir: &Path) -> String {
    if let Some(rest) = host_path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    if Path::new(host_path).is_absolute() {
        host_path.to_string()
    } else {
        agent_dir.join(host_path).display().to_string()
    }
}

/// Resolve a storage URI, making relative sqlite:// paths absolute.
fn resolve_storage_uri(storage: &ResourceId, agent_dir: &Path) -> ResourceId {
    let uri = storage.as_str();

    // sqlite://path → resolve path relative to agent_dir
    if let Some(path) = uri.strip_prefix("sqlite://") {
        if !Path::new(path).is_absolute() {
            let resolved = agent_dir.join(path);
            return ResourceId::new(format!("sqlite://{}", resolved.display()));
        }
    }

    // Other schemes (memory://, s3://, etc.) pass through unchanged
    storage.clone()
}

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Toml(String),
    ExecutableNotFound(String),
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
            runtime = "container"
            executable = "localhost/test-agent:latest"
            object_storage = "sqlite:///data/objects.db"
            vector_storage = "sqlite:///data/vectors.db"

            [requirements]
            services = ["inference"]
        "#;

        let manifest: AgentManifest = toml::from_str(toml).unwrap();

        assert_eq!(manifest.name, "test-agent");
        assert_eq!(manifest.runtime, "container");
        assert_eq!(manifest.executable, "localhost/test-agent:latest");
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
            runtime = "container"
            executable = "localhost/test-agent:latest"

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
            runtime = "container"
            executable = "localhost/test-agent:latest"
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
            runtime = "container"
            executable = "localhost/test-agent:latest"
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
