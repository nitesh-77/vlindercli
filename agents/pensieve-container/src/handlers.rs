//! Intent handlers for the Pensieve agent
//!
//! Each handler implements the logic for one of the supported intents.

use crate::bridge;
use crate::config;
use crate::persistence::{embed_and_store_chunks, get_or_process_content, url_to_key};
use crate::summarize::{chunk_text, generate_summary, SummaryResult};
use crate::util::truncate;

/// Normalize a URL by ensuring it has a scheme
fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with("www.") {
        format!("https://{}", trimmed)
    } else if trimmed.contains('.') && !trimmed.contains(' ') {
        format!("https://{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

/// Convert a URL key back to a displayable URL
fn key_to_display_url(key: &str) -> String {
    let key = key
        .trim_start_matches("/clean/")
        .trim_end_matches(".txt");

    let mut result = key.to_string();

    if result.starts_with("https___") {
        result = result.replacen("https___", "https://", 1);
    } else if result.starts_with("http___") {
        result = result.replacen("http___", "http://", 1);
    }

    for tld in &["com", "org", "net", "io", "dev", "co", "edu", "gov"] {
        let pattern = format!("_{}_", tld);
        let replacement = format!(".{}/", tld);
        result = result.replace(&pattern, &replacement);

        let end_pattern = format!("_{}", tld);
        if result.ends_with(&end_pattern) {
            let len = result.len();
            result = format!("{}.{}", &result[..len - end_pattern.len()], tld);
        }
    }

    result = result.replace('_', "/");

    while result.contains("///") {
        result = result.replace("///", "//");
    }

    result
}

/// Handle PROCESS_URL intent: commit a new web page to memory
pub fn handle_process_url(url: &str) -> Result<String, String> {
    let url = normalize_url(url);
    let url_key = url_to_key(&url);

    // Step 1: Get clean text (from cache or fetch+process)
    eprintln!("[pensieve] step 1: get content for {}", &url);
    let content = get_or_process_content(&url, &url_key)?;
    eprintln!("[pensieve] step 1: done ({}b)", content.len());

    // Step 2: Chunk the content
    let chunks = chunk_text(&content, config::CHUNK_SIZE);
    eprintln!("[pensieve] step 2: {} chunks from {}b content", chunks.len(), content.len());

    // Step 3: Embed and store chunks for semantic search
    eprintln!("[pensieve] step 3: embed and store chunks");
    let embedded_count = embed_and_store_chunks(&url_key, &chunks)?;
    eprintln!("[pensieve] step 3: done ({} embedded)", embedded_count);

    // Step 4: Generate summary
    eprintln!("[pensieve] step 4: generate summary");
    let SummaryResult { briefing, .. } = generate_summary(&chunks)?;
    eprintln!("[pensieve] step 4: done ({}b briefing)", briefing.len());

    // Step 5: Store article-level embedding with Core Argument only
    eprintln!("[pensieve] step 5: store article embedding");
    let article_summary = extract_core_argument(&briefing)
        .unwrap_or_else(|| truncate(&content, 300));
    store_article_embedding(&url_key, &url, &article_summary)?;
    eprintln!("[pensieve] step 5: done");

    Ok(format!(
        "Memory committed\n\
         Source: {}\n\
         Length: {} chars | {} chunks | {} embedded\n\n\
         ---\n\
         Summary:\n{}\n\n\
         ---\n\
         Content preview:\n{}",
        url,
        content.len(),
        chunks.len(),
        embedded_count,
        briefing,
        truncate(&content, 2000)
    ))
}

/// Extract just the Core Argument section from the briefing
fn extract_core_argument(briefing: &str) -> Option<String> {
    let start_marker = "## Core Argument";
    let start = briefing.find(start_marker)?;
    let after_header = &briefing[start + start_marker.len()..];

    let end = after_header.find("\n## ").unwrap_or(after_header.len());
    let content = after_header[..end].trim();

    if content.is_empty() {
        return None;
    }

    Some(content.to_string())
}

/// Store an article-level embedding for search
fn store_article_embedding(url_key: &str, url: &str, summary: &str) -> Result<(), String> {
    let key = format!("article:{}", url_key);
    let embedding = bridge::embed("nomic-embed", summary)?;

    if embedding.starts_with("[error]") {
        return Ok(()); // Silently skip if embedding fails
    }

    let metadata = serde_json::json!({
        "url": url,
        "summary": summary
    }).to_string();

    bridge::store_embedding(&key, &embedding, &metadata)?;
    Ok(())
}

/// Handle LIST_MEMORIES intent: review the index of all stored memories
pub fn handle_list_memories() -> Result<String, String> {
    let files_json = bridge::list_files("/clean")?;

    if files_json.starts_with("[error]") {
        return Ok("No memories found yet. Use a URL to commit your first memory!".to_string());
    }

    let files: Vec<String> = match serde_json::from_str(&files_json) {
        Ok(f) => f,
        Err(_) => {
            files_json
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect()
        }
    };

    if files.is_empty() {
        return Ok("No memories found yet. Use a URL to commit your first memory!".to_string());
    }

    let mut output = format!("Stored Memories ({} total)\n\n", files.len());

    for (i, file) in files.iter().enumerate() {
        let display_url = key_to_display_url(file);
        output.push_str(&format!("{}. {}\n", i + 1, display_url));
    }

    output.push_str("\nTip: Use 'get <url>' to recall a specific memory");

    Ok(output)
}

/// Handle GET_MEMORY intent: recall a specific memory in its entirety
pub fn handle_get_memory(url: &str) -> Result<String, String> {
    let url = normalize_url(url);
    let url_key = url_to_key(&url);
    let clean_cache_path = format!("/clean/{}.txt", url_key);

    let content_bytes = bridge::get_file(&clean_cache_path)?;
    let content = String::from_utf8_lossy(&content_bytes).to_string();

    if content.is_empty() || content.starts_with("[error]") {
        return Ok(format!(
            "Memory not found for: {}\n\n\
             The URL hasn't been committed to memory yet.\n\
             Would you like me to process it now? Just send the full URL.",
            url
        ));
    }

    Ok(format!(
        "Memory Recall\n\
         Source: {}\n\
         Length: {} chars\n\n\
         ---\n\n\
         {}",
        url,
        content.len(),
        content
    ))
}

/// Expand a short query into a more descriptive search query
fn expand_query(query: &str) -> Result<String, String> {
    let prompt = config::QUERY_EXPANSION.replace("{query}", query);
    let expanded = bridge::infer("phi3", &prompt)?;
    let expanded = expanded.trim();

    if expanded.is_empty() || expanded.starts_with("[error]") {
        return Ok(query.to_string());
    }

    Ok(expanded.to_string())
}

/// A search result from the vector store
struct SearchResult {
    source: String,
    preview: String,
    #[allow(dead_code)]
    score: f32,
}

/// Result of finding relevant chunks
struct ChunkSearchResult {
    chunks: Vec<SearchResult>,
}

/// Error types for chunk retrieval
enum ChunkSearchError {
    EmbeddingFailed(String),
    SearchFailed,
    ParseFailed { error: String, raw: String },
}

/// Core retrieval logic: expand query, embed, and search vector store
fn find_relevant_chunks(query: &str, limit: u32) -> Result<ChunkSearchResult, ChunkSearchError> {
    let expanded_query = expand_query(query).map_err(|_| ChunkSearchError::SearchFailed)?;

    let query_embedding = bridge::embed("nomic-embed", &expanded_query)
        .map_err(|_| ChunkSearchError::SearchFailed)?;

    if query_embedding.starts_with("[error]") {
        return Err(ChunkSearchError::EmbeddingFailed(query_embedding));
    }

    let results_json = bridge::search_by_vector(&query_embedding, limit)
        .map_err(|_| ChunkSearchError::SearchFailed)?;

    if results_json.starts_with("[error]") {
        return Err(ChunkSearchError::SearchFailed);
    }

    let chunks = parse_search_results(&results_json).map_err(|e| ChunkSearchError::ParseFailed {
        error: e,
        raw: results_json,
    })?;

    Ok(ChunkSearchResult { chunks })
}

/// An article search result with URL and summary from metadata
struct ArticleSearchResult {
    url: String,
    summary: String,
    score: f32,
}

/// Find relevant articles by searching article-level embeddings
fn find_relevant_articles(query: &str, limit: u32) -> Result<Vec<ArticleSearchResult>, ChunkSearchError> {
    let expanded_query = expand_query(query).map_err(|_| ChunkSearchError::SearchFailed)?;

    let query_embedding = bridge::embed("nomic-embed", &expanded_query)
        .map_err(|_| ChunkSearchError::SearchFailed)?;

    if query_embedding.starts_with("[error]") {
        return Err(ChunkSearchError::EmbeddingFailed(query_embedding));
    }

    let results_json = bridge::search_by_vector(&query_embedding, limit * 3)
        .map_err(|_| ChunkSearchError::SearchFailed)?;

    if results_json.starts_with("[error]") {
        return Err(ChunkSearchError::SearchFailed);
    }

    parse_article_search_results(&results_json, limit as usize)
        .map_err(|e| ChunkSearchError::ParseFailed {
            error: e,
            raw: results_json,
        })
}

/// Parse search results filtering for article-level embeddings only
fn parse_article_search_results(json: &str, limit: usize) -> Result<Vec<ArticleSearchResult>, String> {
    #[derive(serde::Deserialize)]
    struct RawResult {
        #[serde(default)]
        distance: f32,
        #[serde(default)]
        key: String,
        #[serde(default)]
        metadata: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct ArticleMetadata {
        url: String,
        summary: String,
    }

    match serde_json::from_str::<Vec<RawResult>>(json) {
        Ok(raw_results) => {
            let results: Vec<ArticleSearchResult> = raw_results
                .into_iter()
                .filter(|r| r.key.starts_with("article:"))
                .filter_map(|r| {
                    r.metadata.and_then(|meta_str| {
                        serde_json::from_str::<ArticleMetadata>(&meta_str).ok().map(|m| {
                            let score = 1.0 / (1.0 + r.distance);
                            ArticleSearchResult {
                                url: m.url,
                                summary: m.summary,
                                score,
                            }
                        })
                    })
                })
                .take(limit)
                .collect();
            Ok(results)
        }
        Err(e) => Err(format!("Failed to parse article results: {}", e)),
    }
}

/// Handle SEARCH intent: find relevant articles on a topic
pub fn handle_search(query: &str) -> Result<String, String> {
    let result = find_relevant_articles(query, 10);

    match result {
        Err(ChunkSearchError::EmbeddingFailed(err)) => Ok(format!(
            "Failed to process search query: {}\nError: {}",
            query, err
        )),
        Err(ChunkSearchError::SearchFailed) => Ok(format!(
            "Search: \"{}\"\n\n\
             No relevant articles found. Try:\n\
             - Committing more articles to memory\n\
             - Using different search terms",
            query
        )),
        Err(ChunkSearchError::ParseFailed { error, raw }) => Ok(format!(
            "Search: \"{}\"\n\nError parsing results: {}\nRaw response: {}",
            query, error, truncate(&raw, 200)
        )),
        Ok(articles) => {
            if articles.is_empty() {
                return Ok(format!(
                    "Search: \"{}\"\n\n\
                     No relevant articles found for this query.\n\n\
                     Note: Only articles committed after this update have searchable summaries.\n\
                     Try re-committing your articles to enable article-level search.",
                    query
                ));
            }

            let mut output = format!(
                "Search: \"{}\"\nFound {} relevant article{}\n\n",
                query,
                articles.len(),
                if articles.len() == 1 { "" } else { "s" }
            );

            for (i, article) in articles.iter().enumerate() {
                output.push_str(&format!(
                    "{}. {} (relevance: {:.0}%)\n   {}\n\n",
                    i + 1,
                    article.url,
                    article.score * 100.0,
                    article.summary
                ));
            }

            output.push_str("Use \"get <url>\" to read the full article");

            Ok(output)
        }
    }
}

/// Handle QUESTION intent: synthesize an answer from stored memories
pub fn handle_question(query: &str) -> Result<String, String> {
    let result = find_relevant_chunks(query, 5);

    match result {
        Err(ChunkSearchError::EmbeddingFailed(err)) => Ok(format!(
            "Failed to process question: {}\nError: {}",
            query, err
        )),
        Err(ChunkSearchError::SearchFailed) => Ok(format!(
            "Question: \"{}\"\n\n\
             I don't have any relevant memories to answer this question.\n\
             Try committing some articles to memory first!",
            query
        )),
        Err(ChunkSearchError::ParseFailed { error, raw }) => Ok(format!(
            "Question: \"{}\"\n\nError retrieving memories: {}\nRaw response: {}",
            query, error, truncate(&raw, 200)
        )),
        Ok(ChunkSearchResult { chunks }) => {
            if chunks.is_empty() {
                return Ok(format!(
                    "Question: \"{}\"\n\n\
                     I don't have any relevant memories to answer this question.\n\
                     Try committing some articles about this topic!",
                    query
                ));
            }

            let context = chunks
                .iter()
                .map(|c| format!("[From {}]\n{}", c.source, c.preview))
                .collect::<Vec<_>>()
                .join("\n\n---\n\n");

            let answer = generate_answer(query, &context)?;

            let sources: Vec<_> = chunks.iter().map(|c| c.source.clone()).collect();
            let unique_sources: Vec<_> = sources
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            Ok(format!(
                "Question: \"{}\"\n\n{}\n\n---\nSources: {}",
                query,
                answer,
                unique_sources.join(", ")
            ))
        }
    }
}

/// Generate a synthesized answer from context using the LLM
fn generate_answer(question: &str, context: &str) -> Result<String, String> {
    let prompt = config::ANSWER_GENERATION
        .replace("{context}", context)
        .replace("{question}", question);

    let answer = bridge::infer("phi3", &prompt)?;

    if answer.starts_with("[error]") {
        return Ok("I encountered an error while generating the answer.".to_string());
    }

    Ok(answer.trim().to_string())
}

/// Handle UNKNOWN intent: explain what the agent can do
pub fn handle_unknown() -> Result<String, String> {
    Ok(r#"I'm not sure what you'd like to do.

I'm Pensieve, a memory system for web articles. I can help you:

**Save a URL to memory**
   Just paste a link, or say "save <url>"
   Example: https://example.com/article

**List your saved memories**
   "show my memories" or "what have I saved?"

**Recall a specific memory**
   "get <url>" or "recall the article from example.com"

**Find relevant articles**
   "search for <topic>" or "what do I have on <topic>?"

**Ask a question** (synthesized answer)
   "what is great work?"
   "how can I be more productive?"

Try one of these, or paste a URL to get started!"#
        .to_string())
}

/// Parse the JSON search results from the host
fn parse_search_results(json: &str) -> Result<Vec<SearchResult>, String> {
    #[derive(serde::Deserialize)]
    struct RawResult {
        #[serde(default)]
        distance: f32,
        #[serde(default)]
        metadata: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct Metadata {
        #[serde(default)]
        url_key: String,
        #[serde(default)]
        preview: String,
    }

    match serde_json::from_str::<Vec<RawResult>>(json) {
        Ok(raw_results) => {
            let results: Vec<SearchResult> = raw_results
                .into_iter()
                .filter_map(|r| {
                    r.metadata.and_then(|meta_str| {
                        serde_json::from_str::<Metadata>(&meta_str).ok().map(|m| {
                            let score = 1.0 / (1.0 + r.distance);
                            SearchResult {
                                source: key_to_display_url(&m.url_key),
                                preview: m.preview,
                                score,
                            }
                        })
                    })
                })
                .collect();
            Ok(results)
        }
        Err(e) => Err(format!("Failed to parse search results: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_preserves_https() {
        assert_eq!(
            normalize_url("https://example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn normalize_url_preserves_http() {
        assert_eq!(
            normalize_url("http://example.com"),
            "http://example.com"
        );
    }

    #[test]
    fn normalize_url_adds_scheme_to_www() {
        assert_eq!(
            normalize_url("www.example.com/path"),
            "https://www.example.com/path"
        );
    }

    #[test]
    fn normalize_url_adds_scheme_to_domain() {
        assert_eq!(
            normalize_url("example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn normalize_url_trims_whitespace() {
        assert_eq!(
            normalize_url("  https://example.com  "),
            "https://example.com"
        );
    }

    #[test]
    fn parse_search_results_handles_valid_json() {
        let json = r#"[
            {"distance": 1.0, "key": "test:chunk:0", "metadata": "{\"url_key\":\"https___example_com\",\"preview\":\"test preview\"}"}
        ]"#;
        let results = parse_search_results(json).unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 0.5).abs() < 0.01);
        assert!(results[0].source.contains("example"));
        assert_eq!(results[0].preview, "test preview");
    }

    #[test]
    fn parse_search_results_handles_empty_array() {
        let results = parse_search_results("[]").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn parse_search_results_handles_invalid_json() {
        let result = parse_search_results("not json");
        assert!(result.is_err());
    }

    #[test]
    fn key_to_display_url_reconstructs_https() {
        let key = "https___example_com_path_to_article";
        let url = key_to_display_url(key);
        assert!(url.starts_with("https://"));
        assert!(url.contains("example.com"));
    }

    #[test]
    fn key_to_display_url_handles_clean_prefix() {
        let key = "/clean/https___paulgraham_com_greatwork_html.txt";
        let url = key_to_display_url(key);
        assert!(url.starts_with("https://"));
        assert!(url.contains("paulgraham.com"));
    }

    #[test]
    fn parse_article_search_results_filters_by_prefix() {
        let json = r#"[
            {"distance": 1.0, "key": "article:https___example_com", "metadata": "{\"url\":\"https://example.com\",\"summary\":\"A great article about testing.\"}"},
            {"distance": 2.0, "key": "https___example_com:chunk:0", "metadata": "{\"url_key\":\"https___example_com\",\"preview\":\"chunk text\"}"},
            {"distance": 1.5, "key": "article:https___other_com", "metadata": "{\"url\":\"https://other.com\",\"summary\":\"Another summary.\"}"}
        ]"#;
        let results = parse_article_search_results(json, 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[1].url, "https://other.com");
    }

    #[test]
    fn parse_article_search_results_respects_limit() {
        let json = r#"[
            {"distance": 1.0, "key": "article:a", "metadata": "{\"url\":\"a\",\"summary\":\"a\"}"},
            {"distance": 2.0, "key": "article:b", "metadata": "{\"url\":\"b\",\"summary\":\"b\"}"},
            {"distance": 3.0, "key": "article:c", "metadata": "{\"url\":\"c\",\"summary\":\"c\"}"}
        ]"#;
        let results = parse_article_search_results(json, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn extract_core_argument_finds_section() {
        let briefing = r#"## Core Argument
The article argues that great work requires dedication.

## Key Insights
- Point one"#;
        let result = extract_core_argument(briefing);
        assert!(result.is_some());
        let summary = result.unwrap();
        assert!(summary.contains("great work"));
        assert!(!summary.contains("Key Insights"));
    }

    #[test]
    fn extract_core_argument_returns_none_for_missing() {
        let briefing = "Just some text without sections.";
        assert!(extract_core_argument(briefing).is_none());
    }
}
