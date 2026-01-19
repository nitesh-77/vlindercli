//! Configuration constants for the Pensieve agent
//!
//! Centralizes all tunable parameters, organized by ADR.

// --- Text cleaning thresholds (ADR 002) ---

/// Minimum words required for a line to be considered substantive content
pub const MIN_WORDS_PER_LINE: usize = 5;

/// Minimum characters for a line to be considered a real paragraph
pub const MIN_PARAGRAPH_CHARS: usize = 150;

// --- Summarization settings (ADR 003) ---

/// Target size for text chunks (in characters)
pub const CHUNK_SIZE: usize = 1000;

/// Maximum chunks to process for summarization (to control LLM costs)
pub const MAX_CHUNKS_TO_SUMMARIZE: usize = 15;

// --- HTML pre-processing selectors (ADR 004) ---

/// CSS selectors for finding main content containers
pub const CONTENT_SELECTORS: &[&str] = &[
    "article",
    "main",
    "[role=\"main\"]",
    ".post-content",
    ".article-content",
    ".entry-content",
    ".content",
];

/// CSS selectors for boilerplate elements to remove
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

/// Text patterns that indicate boilerplate content
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
