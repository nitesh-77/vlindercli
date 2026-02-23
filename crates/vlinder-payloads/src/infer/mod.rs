//! Inference service payloads — real SDK types per provider.

#[cfg(feature = "openai")]
pub mod openai;

/// Typed inference request payload.
///
/// Each variant carries the real SDK request type for a provider protocol.
/// The variant determines routing — no separate service/operation fields needed.
#[derive(Debug)]
pub enum InferRequest {
    #[cfg(feature = "openai")]
    OpenAi(openai::CreateChatCompletionRequest),
}

/// Typed inference response payload.
#[derive(Debug)]
pub enum InferResponse {
    #[cfg(feature = "openai")]
    OpenAi(openai::CreateChatCompletionResponse),
}

/// Typed inference error — provider errors in their native format.
#[derive(Debug)]
pub enum InferError {
    #[cfg(feature = "openai")]
    OpenAi(String),
}
