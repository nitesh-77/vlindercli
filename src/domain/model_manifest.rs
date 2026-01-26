//! Model manifest as read from model TOML files.

use serde::Deserialize;
use std::path::Path;

/// Model manifest as read from a model.toml file.
#[derive(Clone, Debug, Deserialize)]
pub struct ModelManifest {
    pub name: String,
    #[serde(rename = "type")]
    pub model_type: ModelTypeConfig,
    pub engine: ModelEngineConfig,
    pub model: String,
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
    Llama,
}

impl ModelManifest {
    /// Load a model manifest from a file path.
    ///
    /// Resolves `model` to a URI and validates the file exists.
    pub fn load(path: &Path) -> Result<ModelManifest, ParseError> {
        let content = std::fs::read_to_string(path)?;
        let mut manifest: ModelManifest = toml::from_str(&content)?;

        // Derive model directory from manifest path
        let model_dir = path.parent().unwrap_or(Path::new("."));

        // Resolve model to URI
        manifest.model = resolve_model_uri(&manifest.model, model_dir)?;

        Ok(manifest)
    }
}

/// Resolve a model reference to a URI.
///
/// - If already a URI (contains "://"), return as-is
/// - If absolute path, convert to file:// URI
/// - If relative path, resolve against model_dir and convert to file:// URI
fn resolve_model_uri(model: &str, model_dir: &Path) -> Result<String, ParseError> {
    // Already a URI
    if model.contains("://") {
        // For file:// URIs with relative paths, resolve them
        if let Some(path) = model.strip_prefix("file://") {
            if !Path::new(path).is_absolute() {
                let resolved = model_dir.join(path);
                if !resolved.exists() {
                    return Err(ParseError::ModelNotFound(format!(
                        "model not found: {}",
                        resolved.display()
                    )));
                }
                return Ok(format!("file://{}", resolved.display()));
            }
        }
        return Ok(model.to_string());
    }

    // Resolve path (relative or absolute)
    let model_path = if Path::new(model).is_absolute() {
        Path::new(model).to_path_buf()
    } else {
        model_dir.join(model)
    };

    if !model_path.exists() {
        return Err(ParseError::ModelNotFound(format!(
            "model not found: {}",
            model_path.display()
        )));
    }

    Ok(format!("file://{}", model_path.display()))
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Toml(String),
    ModelNotFound(String),
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
            ParseError::ModelNotFound(e) => write!(f, "{}", e),
        }
    }
}
