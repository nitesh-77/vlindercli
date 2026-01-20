//! Intent handlers for the Pensieve agent
//!
//! Each handler implements the logic for one of the four supported intents.

use extism_pdk::*;

use crate::config::{
    get_prompt, Prompts, CHUNK_SIZE, DEFAULT_ANSWER_GENERATION, DEFAULT_QUERY_EXPANSION,
};
use crate::host::{embed, get_file, infer, list_files, search_by_vector};
use crate::persistence::{embed_and_store_chunks, get_or_process_content, url_to_key};
use crate::summarize::{chunk_text, generate_summary};
use crate::util::truncate;

/// Normalize a URL by ensuring it has a scheme
fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with("www.") {
        format!("https://{}", trimmed)
    } else if trimmed.contains('.') && !trimmed.contains(' ') {
        // Looks like a domain without scheme
        format!("https://{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

/// Convert a URL key back to a displayable URL
/// Since url_to_key converts non-alphanumeric to _, we try to reconstruct
fn key_to_display_url(key: &str) -> String {
    let key = key
        .trim_start_matches("/clean/")
        .trim_end_matches(".txt");

    // Reconstruct the protocol
    let mut result = key.to_string();

    // Replace protocol markers
    if result.starts_with("https___") {
        result = result.replacen("https___", "https://", 1);
    } else if result.starts_with("http___") {
        result = result.replacen("http___", "http://", 1);
    }

    // Common TLD patterns - replace _com_, _org_, etc. with .com/, .org/, etc.
    for tld in &["com", "org", "net", "io", "dev", "co", "edu", "gov"] {
        let pattern = format!("_{}_", tld);
        let replacement = format!(".{}/", tld);
        result = result.replace(&pattern, &replacement);

        // Also handle end of domain (no trailing path)
        let end_pattern = format!("_{}", tld);
        if result.ends_with(&end_pattern) {
            let len = result.len();
            result = format!("{}.{}", &result[..len - end_pattern.len()], tld);
        }
    }

    // Replace remaining underscores with /
    // (This is a heuristic - not perfect but readable)
    result = result.replace('_', "/");

    // Clean up any double slashes (except after protocol)
    while result.contains("///") {
        result = result.replace("///", "//");
    }

    result
}

/// Handle PROCESS_URL intent: commit a new web page to memory
pub fn handle_process_url(url: &str) -> FnResult<String> {
    let url = normalize_url(url);
    let url_key = url_to_key(&url);

    // Step 1: Get clean text (from cache or fetch+process)
    let content = get_or_process_content(&url, &url_key)?;

    // Step 2: Chunk the content
    let chunks = chunk_text(&content, CHUNK_SIZE);

    // Step 3: Embed and store chunks for semantic search
    let embedded_count = embed_and_store_chunks(&url_key, &chunks)?;

    // Step 4: Generate summary
    let summary = generate_summary(&chunks)?;

    Ok(format!(
        "📥 Memory committed\n\
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
        summary,
        truncate(&content, 2000)
    ))
}

/// Handle LIST_MEMORIES intent: review the index of all stored memories
pub fn handle_list_memories() -> FnResult<String> {
    // List files in the /clean directory (where processed content is stored)
    let files_json = unsafe { list_files("/clean".to_string())? };

    if files_json.starts_with("[error]") {
        return Ok("📭 No memories found yet. Use a URL to commit your first memory!".to_string());
    }

    // Parse the file list
    let files: Vec<String> = match serde_json::from_str(&files_json) {
        Ok(f) => f,
        Err(_) => {
            // Try to handle as newline-separated list
            files_json
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect()
        }
    };

    if files.is_empty() {
        return Ok("📭 No memories found yet. Use a URL to commit your first memory!".to_string());
    }

    // Format the list nicely
    let mut output = format!("📚 Stored Memories ({} total)\n\n", files.len());

    for (i, file) in files.iter().enumerate() {
        // Convert the cache key back to a readable URL
        let display_url = key_to_display_url(file);
        output.push_str(&format!("{}. {}\n", i + 1, display_url));
    }

    output.push_str("\n💡 Tip: Use 'get <url>' to recall a specific memory");

    Ok(output)
}

/// Handle GET_MEMORY intent: recall a specific memory in its entirety
pub fn handle_get_memory(url: &str) -> FnResult<String> {
    let url = normalize_url(url);
    let url_key = url_to_key(&url);
    let clean_cache_path = format!("/clean/{}.txt", url_key);

    // Try to get the cached content
    let content_bytes = unsafe { get_file(clean_cache_path)? };
    let content = String::from_utf8_lossy(&content_bytes).to_string();

    if content.is_empty() || content.starts_with("[error]") {
        return Ok(format!(
            "🔍 Memory not found for: {}\n\n\
             The URL hasn't been committed to memory yet.\n\
             Would you like me to process it now? Just send the full URL.",
            url
        ));
    }

    Ok(format!(
        "📖 Memory Recall\n\
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
///
/// This improves semantic search by giving the embedding model more context.
fn expand_query(query: &str) -> FnResult<String> {
    let template = get_prompt(|p: &Prompts| &p.query_expansion, DEFAULT_QUERY_EXPANSION);
    let prompt = template.replace("{query}", query);

    let expanded = unsafe { infer("phi3".to_string(), prompt)? };
    let expanded = expanded.trim();

    // If expansion failed or is empty, fall back to original query
    if expanded.is_empty() || expanded.starts_with("[error]") {
        return Ok(query.to_string());
    }

    Ok(expanded.to_string())
}

/// Result of finding relevant chunks - includes expanded query for display
struct ChunkSearchResult {
    chunks: Vec<SearchResult>,
    expanded_query: String,
}

/// Error types for chunk retrieval
enum ChunkSearchError {
    EmbeddingFailed(String),
    SearchFailed,
    ParseFailed { error: String, raw: String },
}

/// Core retrieval logic: expand query, embed, and search vector store
///
/// This is the shared foundation for both SEARCH (show passages) and QUESTION (synthesize answer).
fn find_relevant_chunks(query: &str, limit: u32) -> Result<ChunkSearchResult, ChunkSearchError> {
    // Step 1: Expand the query for better semantic matching
    let expanded_query = expand_query(query).map_err(|_| ChunkSearchError::SearchFailed)?;

    // Step 2: Generate embedding for the expanded query
    let query_embedding = unsafe {
        embed("nomic-embed".to_string(), expanded_query.clone())
            .map_err(|_| ChunkSearchError::SearchFailed)?
    };

    if query_embedding.starts_with("[error]") {
        return Err(ChunkSearchError::EmbeddingFailed(query_embedding));
    }

    // Step 3: Search for similar vectors
    let results_json = unsafe {
        search_by_vector(query_embedding, limit).map_err(|_| ChunkSearchError::SearchFailed)?
    };

    if results_json.starts_with("[error]") {
        return Err(ChunkSearchError::SearchFailed);
    }

    // Step 4: Parse search results
    let chunks = parse_search_results(&results_json).map_err(|e| ChunkSearchError::ParseFailed {
        error: e,
        raw: results_json,
    })?;

    Ok(ChunkSearchResult {
        chunks,
        expanded_query,
    })
}

/// An article with its best matching chunk
struct ArticleMatch {
    source: String,
    best_score: f32,
    snippet: String,
}

/// Handle SEARCH intent: find relevant articles on a topic
pub fn handle_search(query: &str) -> FnResult<String> {
    // Get more chunks to ensure good article coverage
    let result = find_relevant_chunks(query, 20);

    match result {
        Err(ChunkSearchError::EmbeddingFailed(err)) => Ok(format!(
            "❌ Failed to process search query: {}\n\
             Error: {}",
            query, err
        )),
        Err(ChunkSearchError::SearchFailed) => Ok(format!(
            "🔍 Search: \"{}\"\n\n\
             No relevant articles found. Try:\n\
             - Committing more articles to memory\n\
             - Using different search terms",
            query
        )),
        Err(ChunkSearchError::ParseFailed { error, raw }) => Ok(format!(
            "🔍 Search: \"{}\"\n\n\
             ⚠️ Error parsing results: {}\n\
             Raw response: {}",
            query,
            error,
            truncate(&raw, 200)
        )),
        Ok(ChunkSearchResult {
            chunks,
            expanded_query,
        }) => {
            if chunks.is_empty() {
                return Ok(format!(
                    "🔍 Search: \"{}\"\n\n\
                     No relevant articles found for this query.",
                    query
                ));
            }

            // Aggregate chunks by article, keeping best score and snippet
            let mut articles: std::collections::HashMap<String, ArticleMatch> =
                std::collections::HashMap::new();

            for chunk in chunks {
                articles
                    .entry(chunk.source.clone())
                    .and_modify(|existing| {
                        if chunk.score > existing.best_score {
                            existing.best_score = chunk.score;
                            existing.snippet = truncate(&chunk.preview, 150);
                        }
                    })
                    .or_insert(ArticleMatch {
                        source: chunk.source,
                        best_score: chunk.score,
                        snippet: truncate(&chunk.preview, 150),
                    });
            }

            // Sort by score descending
            let mut sorted: Vec<_> = articles.into_values().collect();
            sorted.sort_by(|a, b| b.best_score.partial_cmp(&a.best_score).unwrap());

            // Format output
            let mut output = format!(
                "🔍 Search: \"{}\"\n\
                 Found {} relevant article{}\n\n",
                query,
                sorted.len(),
                if sorted.len() == 1 { "" } else { "s" }
            );

            for (i, article) in sorted.iter().enumerate() {
                output.push_str(&format!(
                    "{}. {} (relevance: {:.0}%)\n   {}\n\n",
                    i + 1,
                    article.source,
                    article.best_score * 100.0,
                    article.snippet
                ));
            }

            output.push_str("💡 Use \"get <url>\" to read the full article");

            Ok(output)
        }
    }
}

/// Handle QUESTION intent: synthesize an answer from stored memories
pub fn handle_question(query: &str) -> FnResult<String> {
    let result = find_relevant_chunks(query, 5);

    match result {
        Err(ChunkSearchError::EmbeddingFailed(err)) => Ok(format!(
            "❌ Failed to process question: {}\n\
             Error: {}",
            query, err
        )),
        Err(ChunkSearchError::SearchFailed) => Ok(format!(
            "❓ Question: \"{}\"\n\n\
             I don't have any relevant memories to answer this question.\n\
             Try committing some articles to memory first!",
            query
        )),
        Err(ChunkSearchError::ParseFailed { error, raw }) => Ok(format!(
            "❓ Question: \"{}\"\n\n\
             ⚠️ Error retrieving memories: {}\n\
             Raw response: {}",
            query,
            error,
            truncate(&raw, 200)
        )),
        Ok(ChunkSearchResult {
            chunks,
            expanded_query: _,
        }) => {
            if chunks.is_empty() {
                return Ok(format!(
                    "❓ Question: \"{}\"\n\n\
                     I don't have any relevant memories to answer this question.\n\
                     Try committing some articles about this topic!",
                    query
                ));
            }

            // Build context from relevant chunks
            let context = chunks
                .iter()
                .map(|c| format!("[From {}]\n{}", c.source, c.preview))
                .collect::<Vec<_>>()
                .join("\n\n---\n\n");

            // Generate synthesized answer
            let answer = generate_answer(query, &context)?;

            // Format output with sources
            let sources: Vec<_> = chunks.iter().map(|c| c.source.clone()).collect();
            let unique_sources: Vec<_> = sources
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            Ok(format!(
                "❓ Question: \"{}\"\n\n\
                 {}\n\n\
                 ---\n\
                 📚 Sources: {}",
                query,
                answer,
                unique_sources.join(", ")
            ))
        }
    }
}

/// Generate a synthesized answer from context using the LLM
fn generate_answer(question: &str, context: &str) -> FnResult<String> {
    let template = get_prompt(|p: &Prompts| &p.answer_generation, DEFAULT_ANSWER_GENERATION);
    let prompt = template
        .replace("{context}", context)
        .replace("{question}", question);

    let answer = unsafe { infer("phi3".to_string(), prompt)? };

    if answer.starts_with("[error]") {
        return Ok("I encountered an error while generating the answer.".to_string());
    }

    Ok(answer.trim().to_string())
}

/// Handle UNKNOWN intent: explain what the agent can do
pub fn handle_unknown() -> FnResult<String> {
    Ok(r#"🤔 I'm not sure what you'd like to do.

I'm Pensieve, a memory system for web articles. I can help you:

📥 **Save a URL to memory**
   Just paste a link, or say "save <url>"
   Example: https://example.com/article

📚 **List your saved memories**
   "show my memories" or "what have I saved?"

📖 **Recall a specific memory**
   "get <url>" or "recall the article from example.com"

🔍 **Find relevant articles**
   "search for <topic>" or "what do I have on <topic>?"
   Returns a ranked list of articles to explore

❓ **Ask a question** (synthesized answer)
   "what is great work?"
   "how can I be more productive?"
   "explain the key ideas about writing"

Try one of these, or paste a URL to get started!"#
        .to_string())
}

/// A search result from the vector store
struct SearchResult {
    source: String,
    preview: String,
    score: f32,
}

/// Parse the JSON search results from the host
fn parse_search_results(json: &str) -> Result<Vec<SearchResult>, String> {
    // Actual format: [{"distance": 17.1, "key": "...", "metadata": "{\"url_key\":...}"}]
    // Note: metadata is a JSON STRING, not a nested object!
    #[derive(serde::Deserialize)]
    struct RawResult {
        #[serde(default)]
        distance: f32,
        #[serde(default)]
        metadata: Option<String>, // This is a JSON string, not an object
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
                    // Parse the metadata JSON string
                    r.metadata.and_then(|meta_str| {
                        serde_json::from_str::<Metadata>(&meta_str).ok().map(|m| {
                            // Convert distance to similarity score (lower distance = higher score)
                            // Using 1/(1+distance) to normalize to 0-1 range
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
        // Note: metadata is a JSON string, not nested object
        let json = r#"[
            {"distance": 1.0, "key": "test:chunk:0", "metadata": "{\"url_key\":\"https___example_com\",\"preview\":\"test preview\"}"}
        ]"#;
        let results = parse_search_results(json).unwrap();
        assert_eq!(results.len(), 1);
        // score = 1/(1+distance) = 1/2 = 0.5
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
}
