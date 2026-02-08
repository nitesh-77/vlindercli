//! Persistence layer for caching and embeddings (ADR 005)
//!
//! Handles URL caching (HTML and clean text) and semantic search embeddings.

use readability::extractor;

use crate::bridge;
use crate::html::preprocess_html;
use crate::text::clean_text;
use crate::util::truncate;

/// Get clean content from cache or process from scratch
pub fn get_or_process_content(url: &str, url_key: &str) -> Result<String, String> {
    let clean_cache_path = format!("/clean/{}.txt", url_key);

    // Check clean text cache first
    let cached = bridge::get_file(&clean_cache_path)?;
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
        Err(e) => return Err(format!("readability extraction failed: {}", e)),
    };

    // Clean text (ADR 002)
    let content = clean_text(&extracted);

    // Cache the clean content
    let _ = bridge::put_file(&clean_cache_path, content.as_bytes())?;

    Ok(content)
}

/// Get raw HTML from cache or fetch from network
pub fn get_or_fetch_html(url: &str, url_key: &str) -> Result<String, String> {
    let html_cache_path = format!("/html/{}.html", url_key);

    // Check HTML cache
    let cached = bridge::get_file(&html_cache_path)?;
    let cached_str = String::from_utf8_lossy(&cached).to_string();

    if !cached_str.is_empty() && !cached_str.starts_with("[error]") {
        return Ok(cached_str);
    }

    // Fetch from network (container has direct network access)
    let html = bridge::fetch_url(url)?;

    // Cache the HTML
    let _ = bridge::put_file(&html_cache_path, html.as_bytes())?;

    Ok(html)
}

/// Embed and store chunks for semantic search
pub fn embed_and_store_chunks(url_key: &str, chunks: &[String]) -> Result<usize, String> {
    let mut stored_count = 0;

    eprintln!("[pensieve] embedding {} chunks", chunks.len());

    for (i, chunk) in chunks.iter().enumerate() {
        // Skip very short chunks
        if chunk.len() < 50 {
            eprintln!("[pensieve] chunk {}: skipped ({}b < 50)", i, chunk.len());
            continue;
        }

        // Generate embedding
        eprintln!("[pensieve] chunk {}/{}: embedding ({}b)", i, chunks.len(), chunk.len());
        let embedding = bridge::embed("nomic-embed", chunk)?;

        if embedding.starts_with("[error]") {
            eprintln!("[pensieve] chunk {}: embed error: {}", i, &embedding[..embedding.len().min(100)]);
            continue;
        }
        eprintln!("[pensieve] chunk {}: embedded ({}b response)", i, embedding.len());

        // Store with metadata
        let key = format!("{}:chunk:{}", url_key, i);
        let metadata = format!(
            r#"{{"url_key":"{}","chunk_index":{},"preview":"{}"}}"#,
            url_key,
            i,
            truncate(chunk, 100).replace('"', "'")
        );

        eprintln!("[pensieve] chunk {}: storing vector", i);
        let result = bridge::store_embedding(&key, &embedding, &metadata)?;
        if !result.starts_with("[error]") {
            stored_count += 1;
        }
        eprintln!("[pensieve] chunk {}: done (total stored: {})", i, stored_count);
    }

    Ok(stored_count)
}

/// Convert URL to safe cache key
pub fn url_to_key(url: &str) -> String {
    url.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_to_key_replaces_special_chars() {
        let url = "https://example.com/path?query=value";
        let key = url_to_key(url);
        assert!(!key.contains(':'));
        assert!(!key.contains('/'));
        assert!(!key.contains('?'));
        assert!(!key.contains('='));
    }

    #[test]
    fn url_to_key_preserves_alphanumeric() {
        let url = "https://example.com";
        let key = url_to_key(url);
        assert!(key.contains("https"));
        assert!(key.contains("example"));
        assert!(key.contains("com"));
    }

    #[test]
    fn url_to_key_handles_empty() {
        assert_eq!(url_to_key(""), "");
    }

    #[test]
    fn url_to_key_consistent() {
        let url = "https://example.com/article";
        let key1 = url_to_key(url);
        let key2 = url_to_key(url);
        assert_eq!(key1, key2);
    }
}
