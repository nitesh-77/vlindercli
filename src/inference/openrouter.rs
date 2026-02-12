//! OpenRouter inference engine - OpenAI-compatible HTTP client for cloud LLMs.

use serde::{Deserialize, Serialize};

use crate::domain::{InferenceEngine, InferenceResult};

/// Inference engine that calls OpenRouter's OpenAI-compatible API.
pub struct OpenRouterInferenceEngine {
    endpoint: String,
    api_key: String,
    model: String,
}

impl OpenRouterInferenceEngine {
    /// Create a new OpenRouter inference engine.
    ///
    /// - `endpoint`: API base URL (e.g., "https://openrouter.ai/api/v1")
    /// - `api_key`: Bearer token for authentication
    /// - `model`: Model identifier (e.g., "anthropic/claude-sonnet-4-20250514")
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

impl InferenceEngine for OpenRouterInferenceEngine {
    fn infer(&self, prompt: &str, max_tokens: u32) -> Result<InferenceResult, String> {
        let url = format!("{}/chat/completions", self.endpoint);

        let request = ChatCompletionRequest {
            model: &self.model,
            messages: vec![ChatMessage { role: "user", content: prompt }],
            max_tokens,
        };

        let mut response = ureq::post(&url)
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(&request)
            .map_err(|e| format!("openrouter request failed: {}", e))?;

        let body: ChatCompletionResponse = response
            .body_mut()
            .read_json()
            .map_err(|e| format!("failed to parse openrouter response: {}", e))?;

        let text = body.choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| "openrouter returned no choices".to_string())?;

        let (tokens_input, tokens_output) = body.usage
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or((0, 0));

        Ok(InferenceResult { text, tokens_input, tokens_output })
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    max_tokens: u32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_engine_with_all_fields() {
        let engine = OpenRouterInferenceEngine::new(
            "https://openrouter.ai/api/v1",
            "sk-test-key",
            "anthropic/claude-sonnet-4-20250514",
        );
        assert_eq!(engine.endpoint, "https://openrouter.ai/api/v1");
        assert_eq!(engine.api_key, "sk-test-key");
        assert_eq!(engine.model, "anthropic/claude-sonnet-4-20250514");
    }
}
