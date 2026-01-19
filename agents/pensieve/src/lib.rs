//! Pensieve Agent - Extract and recall web content
//!
//! Like Dumbledore's pensieve: store memories (articles) for later review.

use extism_pdk::*;
use readability::extractor;

// Text cleaning thresholds (ADR 002)
const MIN_WORDS_PER_LINE: usize = 5;
const MIN_PARAGRAPH_CHARS: usize = 150;

/// Common boilerplate patterns found in extracted web content
const BOILERPLATE_PATTERNS: &[&str] = &[
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

/// Fetch and process a URL
#[plugin_fn]
pub fn process(url: String) -> FnResult<String> {
    // Fetch the page
    let req = HttpRequest::new(&url);
    let res = http::request::<()>(&req, None)?;
    let html = String::from_utf8(res.body().to_vec())?;

    // Extract article content using Mozilla Readability algorithm (ADR 001)
    let url_parsed = url::Url::parse(&url)
        .unwrap_or_else(|_| url::Url::parse("http://example.com").unwrap());

    let extracted = match extractor::extract(&mut html.as_bytes(), &url_parsed) {
        Ok(product) => {
            format!("{}\n\n{}", product.title, product.text)
        }
        Err(e) => {
            return Ok(format!("Extraction failed: {}", e));
        }
    };

    // Clean extracted text (ADR 002)
    let content = clean_text(&extracted);

    Ok(format!(
        "Source: {}\nLength: {} chars\n\n---\n{}",
        url,
        content.len(),
        content
    ))
}

// --- Text Cleaning (ADR 002) ---

/// Main cleaning function - applies all heuristics in sequence
fn clean_text(text: &str) -> String {
    // Step 1: Remove lines that are pure boilerplate
    let without_boilerplate = remove_boilerplate_lines(text);

    // Step 2: Prune short lines from the beginning
    let pruned = prune_short_leading_lines(&without_boilerplate);

    // Step 3: Find and start from the first "real" paragraph
    find_first_real_paragraph(&pruned)
}

/// Remove lines that match common boilerplate patterns
fn remove_boilerplate_lines(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            let trimmed = lower.trim();

            // Keep empty lines (paragraph separators)
            if trimmed.is_empty() {
                return true;
            }

            // Remove lines that match boilerplate patterns
            !BOILERPLATE_PATTERNS
                .iter()
                .any(|pattern| trimmed == *pattern || trimmed.starts_with(pattern))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Remove short lines from the beginning until we hit substantial content
fn prune_short_leading_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();

    let start_idx = lines
        .iter()
        .position(|line| line.split_whitespace().count() >= MIN_WORDS_PER_LINE)
        .unwrap_or(0);

    lines[start_idx..].join("\n")
}

/// Find the first "real" paragraph - a line exceeding threshold ending with punctuation
fn find_first_real_paragraph(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();

    let start_idx = lines.iter().position(|line| {
        let trimmed = line.trim();
        trimmed.len() >= MIN_PARAGRAPH_CHARS
            && (trimmed.ends_with('.') || trimmed.ends_with('?') || trimmed.ends_with('!'))
    });

    match start_idx {
        Some(idx) => lines[idx..].join("\n"),
        // If no "real" paragraph found, return as-is (don't discard everything)
        None => text.to_string(),
    }
}
