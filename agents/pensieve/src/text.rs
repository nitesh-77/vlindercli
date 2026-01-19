//! Text cleaning functions (ADR 002)
//!
//! Removes boilerplate content and identifies substantive paragraphs.

use crate::config::{BOILERPLATE_PATTERNS, MIN_PARAGRAPH_CHARS, MIN_WORDS_PER_LINE};

/// Clean extracted text by removing boilerplate and finding real content
pub fn clean_text(text: &str) -> String {
    let without_boilerplate = remove_boilerplate_lines(text);
    let pruned = prune_short_leading_lines(&without_boilerplate);
    find_first_real_paragraph(&pruned)
}

/// Remove lines that match known boilerplate patterns
pub fn remove_boilerplate_lines(text: &str) -> String {
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

/// Remove short leading lines that are likely navigation/header cruft
pub fn prune_short_leading_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start_idx = lines
        .iter()
        .position(|line| line.split_whitespace().count() >= MIN_WORDS_PER_LINE)
        .unwrap_or(0);
    lines[start_idx..].join("\n")
}

/// Find the first line that looks like a real paragraph
pub fn find_first_real_paragraph(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_boilerplate_filters_signin() {
        let text = "Sign in\nThis is real content.\nNewsletter";
        let result = remove_boilerplate_lines(text);
        assert!(!result.to_lowercase().contains("sign in"));
        assert!(result.contains("This is real content."));
        assert!(!result.to_lowercase().contains("newsletter"));
    }

    #[test]
    fn remove_boilerplate_preserves_empty_lines() {
        let text = "First line\n\nThird line";
        let result = remove_boilerplate_lines(text);
        assert_eq!(result, text);
    }

    #[test]
    fn remove_boilerplate_case_insensitive() {
        let text = "SIGN IN\nSign Up\nReal content here";
        let result = remove_boilerplate_lines(text);
        assert!(!result.to_lowercase().contains("sign in"));
        assert!(!result.to_lowercase().contains("sign up"));
        assert!(result.contains("Real content here"));
    }

    #[test]
    fn prune_short_lines_finds_first_long_line() {
        let text = "Home\nAbout\nContact\nThis is a line with more than five words in it.\nMore content";
        let result = prune_short_leading_lines(text);
        assert!(result.starts_with("This is a line"));
        assert!(result.contains("More content"));
    }

    #[test]
    fn prune_short_lines_preserves_all_if_no_long_lines() {
        let text = "Short\nAlso short";
        let result = prune_short_leading_lines(text);
        assert_eq!(result, text);
    }

    #[test]
    fn find_paragraph_detects_sentence_ending() {
        // The paragraph must be >= MIN_PARAGRAPH_CHARS (150) and end with punctuation
        let text = "Short intro\nThis is a much longer paragraph that contains enough characters to be considered a real paragraph. It needs to be at least one hundred and fifty characters to pass the threshold.";
        let result = find_first_real_paragraph(text);
        assert!(result.starts_with("This is a much longer"));
    }

    #[test]
    fn find_paragraph_handles_question_mark() {
        // The question must be >= MIN_PARAGRAPH_CHARS (150) and end with ?
        let text = "Title\nWhat happens when we have a really long question that spans many characters and asks something important about the article content or how things work in practice?";
        let result = find_first_real_paragraph(text);
        assert!(result.starts_with("What happens"));
    }

    #[test]
    fn find_paragraph_returns_all_if_no_match() {
        let text = "Short line\nAnother short one";
        let result = find_first_real_paragraph(text);
        assert_eq!(result, text);
    }

    #[test]
    fn clean_text_full_pipeline() {
        let text = "Sign in\nHome\nAbout\nThis article explores a fascinating topic that deserves careful consideration and spans many words to form a proper sentence.";
        let result = clean_text(text);
        assert!(!result.to_lowercase().contains("sign in"));
        assert!(result.contains("This article explores"));
    }
}
