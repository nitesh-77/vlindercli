//! Map-reduce summarization functions (ADR 003)
//!
//! Implements a chunking and summarization strategy for long articles.

use extism_pdk::*;

use crate::config::{
    get_prompt, Prompts, DEFAULT_DIRECT_SUMMARIZE, DEFAULT_MAP_SUMMARIZE, DEFAULT_REDUCE_SUMMARIES,
    MAX_CHUNKS_TO_SUMMARIZE,
};
use crate::host::infer;

/// Split text into chunks at word boundaries
pub fn chunk_text(text: &str, chunk_size: usize) -> Vec<String> {
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

/// Result of summarization
pub struct SummaryResult {
    pub briefing: String,
    pub key_points: Vec<String>,
}

/// Generate a summary using map-reduce strategy
pub fn generate_summary(chunks: &[String]) -> FnResult<SummaryResult> {
    if chunks.is_empty() {
        return Ok(SummaryResult {
            briefing: "No content to summarize.".to_string(),
            key_points: vec![],
        });
    }

    if chunks.len() <= 2 {
        let combined = chunks.join(" ");
        let briefing = summarize_directly(&combined)?;
        return Ok(SummaryResult {
            briefing,
            key_points: vec![],
        });
    }

    let chunks_to_process = if chunks.len() > MAX_CHUNKS_TO_SUMMARIZE {
        sample_chunks_evenly(chunks, MAX_CHUNKS_TO_SUMMARIZE)
    } else {
        chunks.to_vec()
    };

    let key_points = map_summarize_chunks(&chunks_to_process)?;
    let briefing = reduce_summaries(&key_points)?;

    Ok(SummaryResult { briefing, key_points })
}

/// Sample chunks evenly across the content
pub fn sample_chunks_evenly(chunks: &[String], target_count: usize) -> Vec<String> {
    let step = chunks.len() as f64 / target_count as f64;
    (0..target_count)
        .map(|i| {
            let idx = (i as f64 * step) as usize;
            chunks[idx.min(chunks.len() - 1)].clone()
        })
        .collect()
}

/// Map phase: summarize each chunk individually
fn map_summarize_chunks(chunks: &[String]) -> FnResult<Vec<String>> {
    let mut summaries = Vec::with_capacity(chunks.len());
    let template = get_prompt(|p: &Prompts| &p.map_summarize, DEFAULT_MAP_SUMMARIZE);

    for chunk in chunks.iter() {
        if chunk.len() < 100 {
            continue;
        }

        let prompt = template.replace("{chunk}", chunk);

        let summary = unsafe { infer("phi3".to_string(), prompt)? };
        let cleaned = summary.trim();
        if !cleaned.is_empty() {
            summaries.push(cleaned.to_string());
        }
    }

    Ok(summaries)
}

/// Reduce phase: combine chunk summaries into final briefing
fn reduce_summaries(chunk_summaries: &[String]) -> FnResult<String> {
    let numbered: Vec<String> = chunk_summaries
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {}", i + 1, s))
        .collect();
    let combined = numbered.join("\n");

    let template = get_prompt(|p: &Prompts| &p.reduce_summaries, DEFAULT_REDUCE_SUMMARIES);
    let prompt = template.replace("{points}", &combined);

    let result = unsafe { infer("phi3".to_string(), prompt)? };
    Ok(result)
}

/// Summarize short content directly without map-reduce
fn summarize_directly(text: &str) -> FnResult<String> {
    let template = get_prompt(|p: &Prompts| &p.direct_summarize, DEFAULT_DIRECT_SUMMARIZE);
    let prompt = template.replace("{text}", text);
    let result = unsafe { infer("phi3".to_string(), prompt)? };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CHUNK_SIZE;

    #[test]
    fn chunk_text_respects_word_boundaries() {
        let text = "one two three four five six seven eight nine ten";
        let chunks = chunk_text(text, 20);
        // Each chunk should contain complete words
        for chunk in &chunks {
            assert!(!chunk.starts_with(' '));
            assert!(!chunk.ends_with(' '));
        }
    }

    #[test]
    fn chunk_text_respects_size_limit() {
        let text = "word ".repeat(100);
        let chunks = chunk_text(&text, 50);
        for chunk in &chunks {
            assert!(chunk.len() <= 50 + 10); // Allow some margin for word boundaries
        }
    }

    #[test]
    fn chunk_text_handles_empty() {
        let chunks = chunk_text("", 100);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_text_single_chunk_for_short_text() {
        let text = "short text";
        let chunks = chunk_text(text, 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "short text");
    }

    #[test]
    fn sample_chunks_evenly_selects_spread() {
        let chunks: Vec<String> = (0..20).map(|i| format!("chunk_{}", i)).collect();
        let sampled = sample_chunks_evenly(&chunks, 5);

        assert_eq!(sampled.len(), 5);
        // Should include first chunk
        assert_eq!(sampled[0], "chunk_0");
        // Should spread across the range
        assert!(sampled.iter().any(|s| s.contains("chunk_1")));
    }

    #[test]
    fn sample_chunks_handles_exact_count() {
        let chunks: Vec<String> = (0..5).map(|i| format!("chunk_{}", i)).collect();
        let sampled = sample_chunks_evenly(&chunks, 5);
        assert_eq!(sampled.len(), 5);
    }

    #[test]
    fn chunk_size_constant_is_reasonable() {
        // Sanity check that our constant is reasonable
        assert!(CHUNK_SIZE >= 500);
        assert!(CHUNK_SIZE <= 2000);
    }
}
