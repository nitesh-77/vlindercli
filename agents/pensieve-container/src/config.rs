//! Configuration for the Pensieve container agent
//!
//! Simplified from the WASM version: no runtime prompt loading, just defaults.
//! Default prompts are embedded from src/prompts/*.txt at compile time.

// =============================================================================
// Prompts (embedded at compile time)
// =============================================================================

pub const INTENT_RECOGNITION: &str = include_str!("prompts/intent_recognition.txt");
pub const QUERY_EXPANSION: &str = include_str!("prompts/query_expansion.txt");
pub const ANSWER_GENERATION: &str = include_str!("prompts/answer_generation.txt");
pub const MAP_SUMMARIZE: &str = include_str!("prompts/map_summarize.txt");
pub const REDUCE_SUMMARIES: &str = include_str!("prompts/reduce_summaries.txt");
pub const DIRECT_SUMMARIZE: &str = include_str!("prompts/direct_summarize.txt");

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
