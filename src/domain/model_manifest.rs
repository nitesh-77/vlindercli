//! Model manifest as read from model TOML files.

use serde::Deserialize;
use std::path::Path;

/// Model manifest as read from a model.toml file.
#[derive(Clone, Debug, Deserialize)]
pub struct ModelManifest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub model_type: ModelTypeConfig,
    pub engine: ModelEngineConfig,
    /// Path to the model file (e.g., GGUF).
    pub model_path: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelTypeConfig {
    Inference,
    Embedding,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelEngineConfig {
    Ollama,
    OpenRouter,
}

impl ModelManifest {
    /// Load a model manifest from a file path.
    ///
    /// Resolves `model_path` to a URI and validates the file exists.
    pub fn load(path: &Path) -> Result<ModelManifest, ParseError> {
        let content = std::fs::read_to_string(path)?;
        let mut manifest: ModelManifest = toml::from_str(&content)?;

        // Derive model directory from manifest path
        let model_dir = path.parent().unwrap_or(Path::new("."));

        // Resolve model_path to URI
        manifest.model_path = resolve_model_path_uri(&manifest.model_path, model_dir)?;

        Ok(manifest)
    }
}

/// Resolve a model path reference to a URI.
///
/// - If already a URI (contains "://"), return as-is
/// - If absolute path, convert to file:// URI
/// - If relative path, resolve against model_dir and convert to file:// URI
fn resolve_model_path_uri(model_path: &str, model_dir: &Path) -> Result<String, ParseError> {
    // Already a URI
    if model_path.contains("://") {
        // For file:// URIs with relative paths, resolve them
        if let Some(path) = model_path.strip_prefix("file://") {
            if !Path::new(path).is_absolute() {
                let resolved = model_dir.join(path);
                if !resolved.exists() {
                    return Err(ParseError::ModelPathNotFound(format!(
                        "model file not found: {}",
                        resolved.display()
                    )));
                }
                return Ok(format!("file://{}", resolved.display()));
            }
        }
        return Ok(model_path.to_string());
    }

    // Resolve path (relative or absolute)
    let resolved_path = if Path::new(model_path).is_absolute() {
        Path::new(model_path).to_path_buf()
    } else {
        model_dir.join(model_path)
    };

    if !resolved_path.exists() {
        return Err(ParseError::ModelPathNotFound(format!(
            "model file not found: {}",
            resolved_path.display()
        )));
    }

    Ok(format!("file://{}", resolved_path.display()))
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Toml(String),
    ModelPathNotFound(String),
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

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "{}", e),
            ParseError::Toml(e) => write!(f, "{}", e),
            ParseError::ModelPathNotFound(e) => write!(f, "{}", e),
        }
    }
}
