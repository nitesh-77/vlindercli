//! Intent handlers for the Pensieve agent
//!
//! Each handler implements the logic for one of the four supported intents.

use extism_pdk::*;

use crate::config::CHUNK_SIZE;
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
    let prompt = format!(
        r#"You are a query expansion assistant. Your job is to transform short, ambiguous search queries into richer, more descriptive questions that will help find relevant content.

Rules:
- Expand the query into a descriptive question or phrase (1-2 sentences max)
- Include related concepts, synonyms, and aspects of the topic
- Keep it focused - don't add unrelated tangents
- Output ONLY the expanded query, nothing else

Examples:
User: work
Expanded: What are the key aspects of meaningful work, career development, professional growth, and finding purpose in one's labor?

User: AI
Expanded: What are the concepts, applications, and implications of artificial intelligence, machine learning, and automated systems?

User: writing
Expanded: What are effective techniques for writing, composition, prose style, and communicating ideas clearly through text?

User: startups
Expanded: What are the principles of building startups, entrepreneurship, company formation, and growing early-stage businesses?

User: productivity
Expanded: What are strategies for personal productivity, time management, focus, and getting important work done efficiently?

User: {query}
Expanded:"#
    );

    let expanded = unsafe { infer("phi3".to_string(), prompt)? };
    let expanded = expanded.trim();

    // If expansion failed or is empty, fall back to original query
    if expanded.is_empty() || expanded.starts_with("[error]") {
        return Ok(query.to_string());
    }

    Ok(expanded.to_string())
}

/// Handle SEARCH intent: probe all memories for connections on a topic
pub fn handle_search(query: &str) -> FnResult<String> {
    // Step 1: Expand the query for better semantic matching
    let expanded_query = expand_query(query)?;

    // Step 2: Generate embedding for the expanded query
    let query_embedding = unsafe { embed("nomic-embed".to_string(), expanded_query.clone())? };

    if query_embedding.starts_with("[error]") {
        return Ok(format!(
            "❌ Failed to process search query: {}\n\
             Error: {}",
            query, query_embedding
        ));
    }

    // Step 3: Search for similar vectors (top 10 results)
    let results_json = unsafe { search_by_vector(query_embedding, 10)? };

    if results_json.starts_with("[error]") {
        return Ok(format!(
            "🔍 Search: \"{}\"\n\n\
             No relevant memories found. Try:\n\
             - Committing more articles to memory\n\
             - Using different search terms",
            query
        ));
    }

    // Parse search results
    let results = match parse_search_results(&results_json) {
        Ok(r) => r,
        Err(e) => {
            return Ok(format!(
                "🔍 Search: \"{}\"\n\n\
                 ⚠️ Error parsing results: {}\n\
                 Raw response: {}",
                query,
                e,
                truncate(&results_json, 200)
            ));
        }
    };

    if results.is_empty() {
        return Ok(format!(
            "🔍 Search: \"{}\"\n\
             Expanded to: \"{}\"\n\n\
             No relevant memories found for this query.",
            query,
            truncate(&expanded_query, 100)
        ));
    }

    // Format results
    let mut output = format!(
        "🔍 Search: \"{}\"\n\
         Expanded to: \"{}\"\n\
         Found {} relevant passages\n\n",
        query,
        truncate(&expanded_query, 100),
        results.len()
    );

    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "---\n\
             **Result {}** (score: {:.2})\n\
             Source: {}\n\n\
             {}\n\n",
            i + 1,
            result.score,
            result.source,
            result.preview
        ));
    }

    Ok(output)
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

🔍 **Search across all memories**
   "what do I know about <topic>?"
   "search for <concept>"

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
