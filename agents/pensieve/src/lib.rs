//! Pensieve Agent - Extract and recall web content
//!
//! Like Dumbledore's pensieve: store memories (articles) for later review.
//! Now with persistent memory via caching and embeddings (ADR 005).

use extism_pdk::*;

mod config;
mod host;
mod html;
mod persistence;
mod summarize;
mod text;
mod util;

use config::CHUNK_SIZE;
use persistence::{embed_and_store_chunks, get_or_process_content, url_to_key};
use summarize::{chunk_text, generate_summary};
use util::truncate;

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
