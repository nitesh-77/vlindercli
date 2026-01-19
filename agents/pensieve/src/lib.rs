//! Pensieve Agent - Extract and recall web content
//!
//! Like Dumbledore's pensieve: store memories (articles) for later review.

use extism_pdk::*;
use readability::extractor;

#[host_fn]
extern "ExtismHost" {
    fn infer(model: String, prompt: String) -> String;
}

// Text cleaning thresholds (ADR 002)
const MIN_WORDS_PER_LINE: usize = 5;
const MIN_PARAGRAPH_CHARS: usize = 150;

// Summarization settings (ADR 003)
const CHUNK_SIZE: usize = 1000;
const MAX_CHUNKS_TO_SUMMARIZE: usize = 15;

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

    // Chunk and summarize (ADR 003)
    let chunks = chunk_text(&content, CHUNK_SIZE);
    let summary = generate_summary(&chunks)?;

    Ok(format!(
        "Source: {}\nLength: {} chars | {} chunks\n\n---\nSummary:\n{}\n\n---\nContent preview:\n{}",
        url,
        content.len(),
        chunks.len(),
        summary,
        truncate(&content, 2000)
    ))
}

// --- Summarization (ADR 003) ---

/// Chunk text into smaller pieces
fn chunk_text(text: &str, chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let words: Vec<&str> = text.split_whitespace().collect();

    let mut current_chunk = String::new();
    for word in words {
        if current_chunk.len() + word.len() + 1 > chunk_size && !current_chunk.is_empty() {
            chunks.push(current_chunk.trim().to_string());
            current_chunk = String::new();
        }
        if !current_chunk.is_empty() {
            current_chunk.push(' ');
        }
        current_chunk.push_str(word);
    }

    if !current_chunk.trim().is_empty() {
        chunks.push(current_chunk.trim().to_string());
    }

    chunks
}

/// Map-reduce summarization
fn generate_summary(chunks: &[String]) -> FnResult<String> {
    if chunks.is_empty() {
        return Ok("No content to summarize.".to_string());
    }

    // Short articles: summarize directly
    if chunks.len() <= 2 {
        let combined = chunks.join(" ");
        return summarize_directly(&combined);
    }

    // Sample chunks evenly if too many
    let chunks_to_process = if chunks.len() > MAX_CHUNKS_TO_SUMMARIZE {
        sample_chunks_evenly(chunks, MAX_CHUNKS_TO_SUMMARIZE)
    } else {
        chunks.to_vec()
    };

    // MAP: summarize each chunk
    let chunk_summaries = map_summarize_chunks(&chunks_to_process)?;

    // REDUCE: synthesize into final summary
    reduce_summaries(&chunk_summaries)
}

/// Sample chunks evenly across the article
fn sample_chunks_evenly(chunks: &[String], target_count: usize) -> Vec<String> {
    let step = chunks.len() as f64 / target_count as f64;
    (0..target_count)
        .map(|i| {
            let idx = (i as f64 * step) as usize;
            chunks[idx.min(chunks.len() - 1)].clone()
        })
        .collect()
}

/// Map step: summarize each chunk
fn map_summarize_chunks(chunks: &[String]) -> FnResult<Vec<String>> {
    let mut summaries = Vec::with_capacity(chunks.len());

    for chunk in chunks.iter() {
        if chunk.len() < 100 {
            continue;
        }

        let prompt = format!(
            "What is the main point? Answer in 1-2 sentences:\n\n{}",
            chunk
        );

        let summary = unsafe { infer("phi3".to_string(), prompt)? };

        let cleaned = summary.trim();
        if !cleaned.is_empty() {
            summaries.push(cleaned.to_string());
        }
    }

    Ok(summaries)
}

/// Reduce step: synthesize chunk summaries
fn reduce_summaries(chunk_summaries: &[String]) -> FnResult<String> {
    let numbered: Vec<String> = chunk_summaries
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {}", i + 1, s))
        .collect();
    let combined = numbered.join("\n");

    let prompt = format!(
        "These are key points from an article:\n\n{}\n\nWrite a 3-5 bullet point summary:",
        combined
    );

    let result = unsafe { infer("phi3".to_string(), prompt)? };

    Ok(result)
}

/// Direct summarization for short articles
fn summarize_directly(text: &str) -> FnResult<String> {
    let prompt = format!(
        "Summarize this article in 3-5 bullet points:\n\n{}",
        text
    );

    let result = unsafe { infer("phi3".to_string(), prompt)? };

    Ok(result)
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
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
