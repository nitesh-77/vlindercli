//! Pensieve Agent - Extract and recall web content
//!
//! Like Dumbledore's pensieve: store memories (articles) for later review.
//! Now with persistent memory via caching and embeddings (ADR 005).

use extism_pdk::*;
use readability::extractor;
use scraper::{Html, Selector};

#[host_fn]
extern "ExtismHost" {
    fn infer(model: String, prompt: String) -> String;
    // Persistence layer (ADR 005)
    fn get_file(path: String) -> Vec<u8>;
    fn put_file(path: String, content: Vec<u8>) -> String;
    fn embed(model: String, text: String) -> String;
    fn store_embedding(key: String, vector: String, metadata: String) -> String;
}

// Text cleaning thresholds (ADR 002)
const MIN_WORDS_PER_LINE: usize = 5;
const MIN_PARAGRAPH_CHARS: usize = 150;

// Summarization settings (ADR 003)
const CHUNK_SIZE: usize = 1000;
const MAX_CHUNKS_TO_SUMMARIZE: usize = 15;

// HTML pre-processing selectors (ADR 004)
const CONTENT_SELECTORS: &[&str] = &[
    "article",
    "main",
    "[role=\"main\"]",
    ".post-content",
    ".article-content",
    ".entry-content",
    ".content",
];

const BOILERPLATE_SELECTORS: &[&str] = &[
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

const BOILERPLATE_PATTERNS: &[&str] = &[
    "sign in", "sign up", "log in", "log out", "subscribe", "newsletter",
    "toggle dark mode", "toggle light mode", "open main menu", "close menu",
    "skip to content", "skip to main", "search", "share this", "share on",
    "follow us", "connect with us", "privacy policy", "terms of service",
    "cookie policy", "all rights reserved", "powered by", "advertisement", "sponsored",
];

/// Fetch and process a URL with caching and persistence (ADR 005)
#[plugin_fn]
pub fn process(url: String) -> FnResult<String> {
    let url_key = url_to_key(&url);

    // Step 1: Get clean text (from cache or fetch+process)
    let content = get_or_process_content(&url, &url_key)?;

    // Step 2: Chunk the content
    let chunks = chunk_text(&content, CHUNK_SIZE);

    // Step 3: Embed and store chunks for semantic search (ADR 005)
    let embedded_count = embed_and_store_chunks(&url_key, &chunks)?;

    // Step 4: Generate summary (ADR 003)
    let summary = generate_summary(&chunks)?;

    Ok(format!(
        "Source: {}\nLength: {} chars | {} chunks | {} embedded\n\n---\nSummary:\n{}\n\n---\nContent preview:\n{}",
        url,
        content.len(),
        chunks.len(),
        embedded_count,
        summary,
        truncate(&content, 2000)
    ))
}

// --- Persistence Layer (ADR 005) ---

/// Get clean content from cache or process from scratch
fn get_or_process_content(url: &str, url_key: &str) -> FnResult<String> {
    let clean_cache_path = format!("/clean/{}.txt", url_key);

    // Check clean text cache first
    let cached = unsafe { get_file(clean_cache_path.clone())? };
    let cached_str = String::from_utf8_lossy(&cached).to_string();

    if !cached_str.is_empty() && !cached_str.starts_with("[error]") {
        return Ok(cached_str);
    }

    // Not cached - need to fetch and process
    let raw_html = get_or_fetch_html(url, url_key)?;

    // Pre-process HTML (ADR 004)
    let clean_html = preprocess_html(&raw_html);

    // Extract with readability (ADR 001)
    let url_parsed = url::Url::parse(url)
        .unwrap_or_else(|_| url::Url::parse("http://example.com").unwrap());

    let extracted = match extractor::extract(&mut clean_html.as_bytes(), &url_parsed) {
        Ok(product) => format!("{}\n\n{}", product.title, product.text),
        Err(e) => return Err(e.into()),
    };

    // Clean text (ADR 002)
    let content = clean_text(&extracted);

    // Cache the clean content
    unsafe {
        let _ = put_file(clean_cache_path, content.as_bytes().to_vec())?;
    }

    Ok(content)
}

/// Get raw HTML from cache or fetch from network
fn get_or_fetch_html(url: &str, url_key: &str) -> FnResult<String> {
    let html_cache_path = format!("/html/{}.html", url_key);

    // Check HTML cache
    let cached = unsafe { get_file(html_cache_path.clone())? };
    let cached_str = String::from_utf8_lossy(&cached).to_string();

    if !cached_str.is_empty() && !cached_str.starts_with("[error]") {
        return Ok(cached_str);
    }

    // Fetch from network
    let req = HttpRequest::new(url);
    let res = http::request::<()>(&req, None)?;
    let html = String::from_utf8(res.body().to_vec())?;

    // Cache the HTML
    unsafe {
        let _ = put_file(html_cache_path, html.as_bytes().to_vec())?;
    }

    Ok(html)
}

/// Embed and store chunks for semantic search
fn embed_and_store_chunks(url_key: &str, chunks: &[String]) -> FnResult<usize> {
    let mut stored_count = 0;

    for (i, chunk) in chunks.iter().enumerate() {
        // Skip very short chunks
        if chunk.len() < 50 {
            continue;
        }

        // Generate embedding
        let embedding = unsafe { embed("nomic-embed".to_string(), chunk.clone())? };

        if embedding.starts_with("[error]") {
            continue;
        }

        // Store with metadata
        let key = format!("{}:chunk:{}", url_key, i);
        let metadata = format!(
            r#"{{"url_key":"{}","chunk_index":{},"preview":"{}"}}"#,
            url_key,
            i,
            truncate(chunk, 100).replace('"', "'")
        );

        unsafe {
            let result = store_embedding(key, embedding, metadata)?;
            if !result.starts_with("[error]") {
                stored_count += 1;
            }
        }
    }

    Ok(stored_count)
}

/// Convert URL to safe cache key
fn url_to_key(url: &str) -> String {
    url.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

// --- HTML Pre-processing (ADR 004) ---

fn preprocess_html(raw_html: &str) -> String {
    let mut document = Html::parse_document(raw_html);

    // Strategy 1: Try to find main content container
    for selector_str in CONTENT_SELECTORS {
        if let Ok(selector) = Selector::parse(selector_str) {
            if let Some(element) = document.select(&selector).next() {
                return element.html();
            }
        }
    }

    // Strategy 2: Remove boilerplate elements from DOM
    let mut nodes_to_remove = Vec::new();
    for selector_str in BOILERPLATE_SELECTORS {
        if let Ok(selector) = Selector::parse(selector_str) {
            for element in document.select(&selector) {
                nodes_to_remove.push(element.id());
            }
        }
    }

    for node_id in nodes_to_remove {
        if let Some(mut node) = document.tree.get_mut(node_id) {
            node.detach();
        }
    }

    document.html()
}

// --- Summarization (ADR 003) ---

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

fn generate_summary(chunks: &[String]) -> FnResult<String> {
    if chunks.is_empty() {
        return Ok("No content to summarize.".to_string());
    }

    if chunks.len() <= 2 {
        let combined = chunks.join(" ");
        return summarize_directly(&combined);
    }

    let chunks_to_process = if chunks.len() > MAX_CHUNKS_TO_SUMMARIZE {
        sample_chunks_evenly(chunks, MAX_CHUNKS_TO_SUMMARIZE)
    } else {
        chunks.to_vec()
    };

    let chunk_summaries = map_summarize_chunks(&chunks_to_process)?;
    reduce_summaries(&chunk_summaries)
}

fn sample_chunks_evenly(chunks: &[String], target_count: usize) -> Vec<String> {
    let step = chunks.len() as f64 / target_count as f64;
    (0..target_count)
        .map(|i| {
            let idx = (i as f64 * step) as usize;
            chunks[idx.min(chunks.len() - 1)].clone()
        })
        .collect()
}

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

fn reduce_summaries(chunk_summaries: &[String]) -> FnResult<String> {
    let numbered: Vec<String> = chunk_summaries
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {}", i + 1, s))
        .collect();
    let combined = numbered.join("\n");

    let prompt = format!(
        r#"You are an expert analyst. Based on these key points from an article, create a structured briefing.

KEY POINTS:
{}

Generate a briefing with these sections:

## Core Argument
State the article's central thesis in 2-3 sentences.

## Key Insights
List 3-5 most important takeaways as bullet points.

## Practical Applications
How can a reader apply these ideas? Give 2-3 actionable suggestions.

## Questions Raised
What 2-3 thought-provoking questions does this article raise for further exploration?

Be concise but insightful."#,
        combined
    );

    let result = unsafe { infer("phi3".to_string(), prompt)? };
    Ok(result)
}

fn summarize_directly(text: &str) -> FnResult<String> {
    let prompt = format!("Summarize this article in 3-5 bullet points:\n\n{}", text);
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

fn clean_text(text: &str) -> String {
    let without_boilerplate = remove_boilerplate_lines(text);
    let pruned = prune_short_leading_lines(&without_boilerplate);
    find_first_real_paragraph(&pruned)
}

fn remove_boilerplate_lines(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let trimmed = line.to_lowercase();
            let trimmed = trimmed.trim();
            if trimmed.is_empty() {
                return true;
            }
            !BOILERPLATE_PATTERNS
                .iter()
                .any(|p| trimmed == *p || trimmed.starts_with(p))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn prune_short_leading_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start_idx = lines
        .iter()
        .position(|line| line.split_whitespace().count() >= MIN_WORDS_PER_LINE)
        .unwrap_or(0);
    lines[start_idx..].join("\n")
}

fn find_first_real_paragraph(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start_idx = lines.iter().position(|line| {
        let trimmed = line.trim();
        trimmed.len() >= MIN_PARAGRAPH_CHARS
            && (trimmed.ends_with('.') || trimmed.ends_with('?') || trimmed.ends_with('!'))
    });

    match start_idx {
        Some(idx) => lines[idx..].join("\n"),
        None => text.to_string(),
    }
}
