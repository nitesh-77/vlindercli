//! Intent classification for the Pensieve agent
//!
//! Analyzes user input to determine what action they want to perform.

use extism_pdk::*;
use serde::Deserialize;

use crate::config::{get_prompt, Prompts, DEFAULT_INTENT_RECOGNITION};
use crate::host::infer;

/// The supported user intents
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// Commit a new web page to memory
    ProcessUrl { url: String },
    /// Review the index of all stored memories
    ListMemories,
    /// Recall a specific memory in its entirety
    GetMemory { url: String },
    /// Probe all memories for connections on a topic (shows raw passages)
    Search { query: String },
    /// Ask a question and get a synthesized answer from memories
    Question { query: String },
    /// Unable to determine intent with confidence
    Unknown,
}

/// Raw JSON response from the LLM
#[derive(Debug, Deserialize)]
struct IntentResponse {
    intent: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
}

/// Minimum confidence threshold for accepting an intent
const CONFIDENCE_THRESHOLD: f32 = 0.6;

/// Determine the user's intent from their input
pub fn determine_intent(input: &str) -> FnResult<Intent> {
    // Fast path: detect URLs directly without LLM
    if looks_like_url(input) {
        return Ok(Intent::ProcessUrl {
            url: input.trim().to_string(),
        });
    }

    // Get prompt template (Vlinderfile override or compiled-in default)
    let template = get_prompt(
        |p: &Prompts| &p.intent_recognition,
        DEFAULT_INTENT_RECOGNITION,
    );
    let prompt = template.replace("{input}", input);

    let response = unsafe { infer("phi3".to_string(), prompt)? };

    parse_intent_response(&response, input)
}

/// Check if input looks like a bare URL
fn looks_like_url(input: &str) -> bool {
    let trimmed = input.trim();
    trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || (trimmed.starts_with("www.") && trimmed.contains('.'))
}

/// Parse the LLM's JSON response into an Intent
fn parse_intent_response(response: &str, original_input: &str) -> FnResult<Intent> {
    // Try to extract JSON from the response (LLM might include extra text)
    let json_str = extract_json(response);

    match serde_json::from_str::<IntentResponse>(&json_str) {
        Ok(parsed) => {
            // Check confidence - if below threshold, return Unknown
            let confidence = parsed.confidence.unwrap_or(0.5);
            if confidence < CONFIDENCE_THRESHOLD {
                return Ok(Intent::Unknown);
            }

            match parsed.intent.to_uppercase().as_str() {
                "PROCESS_URL" => {
                    let url = parsed
                        .url
                        .or_else(|| extract_url_from_text(original_input))
                        .unwrap_or_else(|| original_input.trim().to_string());
                    Ok(Intent::ProcessUrl { url })
                }
                "LIST_MEMORIES" => Ok(Intent::ListMemories),
                "GET_MEMORY" => {
                    let url = parsed
                        .url
                        .or_else(|| extract_url_from_text(original_input))
                        .unwrap_or_else(|| original_input.trim().to_string());
                    Ok(Intent::GetMemory { url })
                }
                "SEARCH" => {
                    let query = parsed
                        .query
                        .unwrap_or_else(|| original_input.trim().to_string());
                    Ok(Intent::Search { query })
                }
                "QUESTION" => {
                    let query = parsed
                        .query
                        .unwrap_or_else(|| original_input.trim().to_string());
                    Ok(Intent::Question { query })
                }
                "UNKNOWN" | _ => Ok(Intent::Unknown),
            }
        }
        Err(_) => {
            // If JSON parsing fails, try to infer from keywords
            fallback_intent_detection(original_input)
        }
    }
}

/// Extract JSON object from potentially messy LLM output
fn extract_json(text: &str) -> String {
    // Find the first { and last }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if start < end {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}

/// Extract a URL from text
fn extract_url_from_text(text: &str) -> Option<String> {
    for word in text.split_whitespace() {
        if word.starts_with("http://") || word.starts_with("https://") {
            return Some(word.to_string());
        }
    }
    None
}

/// Fallback intent detection using keywords
fn fallback_intent_detection(input: &str) -> FnResult<Intent> {
    let lower = input.to_lowercase();

    // Check for URL first
    if let Some(url) = extract_url_from_text(input) {
        return Ok(Intent::ProcessUrl { url });
    }

    // Check for list keywords
    if lower.contains("list")
        || lower.contains("show all")
        || lower.contains("what have i")
        || lower.contains("my memories")
    {
        return Ok(Intent::ListMemories);
    }

    // Check for get/recall keywords with URL-like content
    if (lower.contains("get") || lower.contains("recall") || lower.contains("retrieve"))
        && (lower.contains(".com") || lower.contains(".org") || lower.contains(".io"))
    {
        // Try to extract something URL-like
        for word in input.split_whitespace() {
            if word.contains('.') && !word.starts_with('.') {
                return Ok(Intent::GetMemory {
                    url: word.to_string(),
                });
            }
        }
    }

    // Check for explicit search keywords (user wants raw passages)
    // Note: "what do i know" specifically requests stored info, so it's SEARCH not QUESTION
    if lower.contains("search")
        || lower.contains("find passages")
        || lower.contains("what do i have")
        || lower.contains("what's in my memories")
        || lower.contains("what do i know")
    {
        return Ok(Intent::Search {
            query: input.trim().to_string(),
        });
    }

    // Check for question patterns (user wants a synthesized answer)
    // Questions typically end with "?" or start with question words
    if lower.ends_with('?')
        || lower.starts_with("what is")
        || lower.starts_with("what are")
        || lower.starts_with("how")
        || lower.starts_with("why")
        || lower.starts_with("explain")
        || lower.starts_with("tell me")
    {
        return Ok(Intent::Question {
            query: input.trim().to_string(),
        });
    }

    // Default to unknown - don't guess
    Ok(Intent::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_url_detects_http() {
        assert!(looks_like_url("http://example.com"));
        assert!(looks_like_url("https://example.com/path"));
        assert!(looks_like_url("  https://example.com  "));
    }

    #[test]
    fn looks_like_url_detects_www() {
        assert!(looks_like_url("www.example.com"));
    }

    #[test]
    fn looks_like_url_rejects_plain_text() {
        assert!(!looks_like_url("hello world"));
        assert!(!looks_like_url("search for AI"));
    }

    #[test]
    fn extract_json_finds_object() {
        let messy = "Here's the JSON: {\"intent\": \"SEARCH\"} hope that helps!";
        assert_eq!(extract_json(messy), "{\"intent\": \"SEARCH\"}");
    }

    #[test]
    fn extract_url_from_text_finds_url() {
        let text = "save this https://example.com/article please";
        assert_eq!(
            extract_url_from_text(text),
            Some("https://example.com/article".to_string())
        );
    }

    #[test]
    fn extract_url_from_text_returns_none_for_no_url() {
        assert_eq!(extract_url_from_text("no url here"), None);
    }

    #[test]
    fn fallback_detects_list_intent() {
        let result = fallback_intent_detection("list all my memories").unwrap();
        assert_eq!(result, Intent::ListMemories);
    }

    #[test]
    fn fallback_detects_search_intent() {
        let result = fallback_intent_detection("what do I know about rust?").unwrap();
        assert!(matches!(result, Intent::Search { .. }));
    }

    #[test]
    fn fallback_detects_url_in_text() {
        let result = fallback_intent_detection("save https://example.com").unwrap();
        assert!(matches!(result, Intent::ProcessUrl { .. }));
    }

    #[test]
    fn fallback_returns_unknown_for_gibberish() {
        let result = fallback_intent_detection("asdfghjkl").unwrap();
        assert_eq!(result, Intent::Unknown);
    }

    #[test]
    fn fallback_returns_unknown_for_greeting() {
        let result = fallback_intent_detection("hello there").unwrap();
        assert_eq!(result, Intent::Unknown);
    }

    // --- parse_intent_response tests ---

    #[test]
    fn parse_intent_response_handles_process_url() {
        let json = r#"{"intent": "PROCESS_URL", "url": "https://example.com", "confidence": 0.9}"#;
        let result = parse_intent_response(json, "save https://example.com").unwrap();
        assert!(matches!(result, Intent::ProcessUrl { url } if url == "https://example.com"));
    }

    #[test]
    fn parse_intent_response_handles_list_memories() {
        let json = r#"{"intent": "LIST_MEMORIES", "confidence": 0.85}"#;
        let result = parse_intent_response(json, "show my memories").unwrap();
        assert_eq!(result, Intent::ListMemories);
    }

    #[test]
    fn parse_intent_response_handles_get_memory() {
        let json = r#"{"intent": "GET_MEMORY", "url": "example.com", "confidence": 0.8}"#;
        let result = parse_intent_response(json, "get example.com").unwrap();
        assert!(matches!(result, Intent::GetMemory { url } if url == "example.com"));
    }

    #[test]
    fn parse_intent_response_handles_search() {
        let json = r#"{"intent": "SEARCH", "query": "machine learning", "confidence": 0.9}"#;
        let result = parse_intent_response(json, "what do I know about ML?").unwrap();
        assert!(matches!(result, Intent::Search { query } if query == "machine learning"));
    }

    #[test]
    fn parse_intent_response_handles_unknown() {
        let json = r#"{"intent": "UNKNOWN", "confidence": 0.95}"#;
        let result = parse_intent_response(json, "hello").unwrap();
        assert_eq!(result, Intent::Unknown);
    }

    #[test]
    fn parse_intent_response_rejects_low_confidence() {
        // Even with a valid intent, low confidence should return Unknown
        let json = r#"{"intent": "SEARCH", "query": "test", "confidence": 0.3}"#;
        let result = parse_intent_response(json, "maybe search?").unwrap();
        assert_eq!(result, Intent::Unknown);
    }

    #[test]
    fn parse_intent_response_threshold_boundary() {
        // Exactly at threshold (0.6) should pass
        let json = r#"{"intent": "LIST_MEMORIES", "confidence": 0.6}"#;
        let result = parse_intent_response(json, "list").unwrap();
        assert_eq!(result, Intent::ListMemories);

        // Just below threshold should fail
        let json_low = r#"{"intent": "LIST_MEMORIES", "confidence": 0.59}"#;
        let result_low = parse_intent_response(json_low, "list").unwrap();
        assert_eq!(result_low, Intent::Unknown);
    }

    #[test]
    fn parse_intent_response_missing_confidence_uses_default() {
        // Missing confidence defaults to 0.5, which is below threshold
        let json = r#"{"intent": "SEARCH", "query": "test"}"#;
        let result = parse_intent_response(json, "test").unwrap();
        assert_eq!(result, Intent::Unknown);
    }

    #[test]
    fn parse_intent_response_extracts_url_from_input() {
        // If LLM doesn't provide URL, extract from original input
        let json = r#"{"intent": "PROCESS_URL", "confidence": 0.9}"#;
        let result = parse_intent_response(json, "save https://fallback.com").unwrap();
        assert!(matches!(result, Intent::ProcessUrl { url } if url == "https://fallback.com"));
    }

    #[test]
    fn parse_intent_response_handles_messy_json() {
        // LLM might include extra text around JSON
        let messy = "Sure! Here's the intent: {\"intent\": \"LIST_MEMORIES\", \"confidence\": 0.9} Hope that helps!";
        let result = parse_intent_response(messy, "show memories").unwrap();
        assert_eq!(result, Intent::ListMemories);
    }

    #[test]
    fn parse_intent_response_fallback_on_invalid_json() {
        // Invalid JSON should trigger fallback detection
        let invalid = "not valid json at all";
        let result = parse_intent_response(invalid, "list my memories").unwrap();
        // Fallback should detect "list" keyword
        assert_eq!(result, Intent::ListMemories);
    }

    #[test]
    fn parse_intent_response_case_insensitive_intent() {
        let json = r#"{"intent": "search", "query": "test", "confidence": 0.9}"#;
        let result = parse_intent_response(json, "search test").unwrap();
        assert!(matches!(result, Intent::Search { .. }));
    }

    // --- QUESTION intent tests ---

    #[test]
    fn parse_intent_response_handles_question() {
        let json = r#"{"intent": "QUESTION", "query": "what is great work?", "confidence": 0.9}"#;
        let result = parse_intent_response(json, "what is great work?").unwrap();
        assert!(matches!(result, Intent::Question { query } if query == "what is great work?"));
    }

    #[test]
    fn fallback_detects_question_with_question_mark() {
        let result = fallback_intent_detection("how can I be more productive?").unwrap();
        assert!(matches!(result, Intent::Question { .. }));
    }

    #[test]
    fn fallback_detects_question_with_explain() {
        let result = fallback_intent_detection("explain the key ideas").unwrap();
        assert!(matches!(result, Intent::Question { .. }));
    }

    #[test]
    fn fallback_detects_question_with_tell_me() {
        let result = fallback_intent_detection("tell me about startups").unwrap();
        assert!(matches!(result, Intent::Question { .. }));
    }

    #[test]
    fn fallback_prefers_search_over_question_for_what_do_i_know() {
        // "what do I know" is explicitly a SEARCH intent (user wants to see what's stored)
        let result = fallback_intent_detection("what do I know about AI?").unwrap();
        assert!(matches!(result, Intent::Search { .. }));
    }
}
