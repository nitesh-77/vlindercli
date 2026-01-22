//! Configuration for the Pensieve agent
//!
//! Loads the agent manifest at runtime for prompt overrides.
//! Default prompts are embedded from src/prompts/*.txt at compile time.

use once_cell::sync::Lazy;
use serde::Deserialize;

use crate::host::get_manifest;

// =============================================================================
// Manifest Structs
// =============================================================================

/// Root configuration from agent manifest
#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub source: Option<String>,
    pub requirements: Requirements,
    #[serde(default)]
    pub prompts: Option<Prompts>,
}

/// Runtime requirements
#[derive(Debug, Deserialize)]
pub struct Requirements {
    pub models: Vec<String>,
    pub host_capabilities: Vec<String>,
}

/// Optional prompt overrides
#[derive(Debug, Default, Deserialize)]
pub struct Prompts {
    pub intent_recognition: Option<String>,
    pub query_expansion: Option<String>,
    pub answer_generation: Option<String>,
    pub map_summarize: Option<String>,
    pub reduce_summaries: Option<String>,
    pub direct_summarize: Option<String>,
}

// =============================================================================
// Config Loader
// =============================================================================

/// Global config, loaded lazily
pub static CONFIG: Lazy<Manifest> = Lazy::new(|| {
    let content = unsafe { get_manifest().expect("Failed to load manifest") };
    toml::from_str(&content).expect("Failed to parse manifest")
});

/// Get prompt: manifest override if present, else compiled-in default
pub fn get_prompt(getter: fn(&Prompts) -> &Option<String>, default: &str) -> String {
    CONFIG
        .prompts
        .as_ref()
        .and_then(|p| getter(p).as_ref())
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

// =============================================================================
// Default Prompts (embedded at compile time)
// =============================================================================

pub const DEFAULT_INTENT_RECOGNITION: &str = include_str!("prompts/intent_recognition.txt");
pub const DEFAULT_QUERY_EXPANSION: &str = include_str!("prompts/query_expansion.txt");
pub const DEFAULT_ANSWER_GENERATION: &str = include_str!("prompts/answer_generation.txt");
pub const DEFAULT_MAP_SUMMARIZE: &str = include_str!("prompts/map_summarize.txt");
pub const DEFAULT_REDUCE_SUMMARIES: &str = include_str!("prompts/reduce_summaries.txt");
pub const DEFAULT_DIRECT_SUMMARIZE: &str = include_str!("prompts/direct_summarize.txt");

// =============================================================================
// Compile-time Constants
// =============================================================================

// --- Text cleaning (ADR 002) ---
pub const MIN_WORDS_PER_LINE: usize = 5;
pub const MIN_PARAGRAPH_CHARS: usize = 150;

// --- Summarization (ADR 003) ---
pub const CHUNK_SIZE: usize = 1000;
pub const MAX_CHUNKS_TO_SUMMARIZE: usize = 15;

// --- HTML selectors (ADR 004) ---
pub const CONTENT_SELECTORS: &[&str] = &[
    "article",
    "main",
    "[role=\"main\"]",
    ".post-content",
    ".article-content",
    ".entry-content",
    ".content",
];

pub const BOILERPLATE_SELECTORS: &[&str] = &[
    "nav",
    "header",
    "footer",
    "aside",
    "[role=\"navigation\"]",
    "[role=\"banner\"]",
    "[role=\"contentinfo\"]",
    ".sidebar",
    ".nav",
    ".menu",
    ".advertisement",
    ".social-share",
    ".comments",
    "#comments",
];

pub const BOILERPLATE_PATTERNS: &[&str] = &[
    "sign in",
    "sign up",
    "log in",
    "log out",
    "subscribe",
    "newsletter",
    "toggle dark mode",
    "toggle light mode",
    "open main menu",
    "close menu",
    "skip to content",
    "skip to main",
    "search",
    "share this",
    "share on",
    "follow us",
    "connect with us",
    "privacy policy",
    "terms of service",
    "cookie policy",
    "all rights reserved",
    "powered by",
    "advertisement",
    "sponsored",
];
