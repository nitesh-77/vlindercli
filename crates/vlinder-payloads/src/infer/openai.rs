//! OpenAI Chat Completion protocol types.
//!
//! Re-exports from `async-openai`. These are the real SDK types —
//! the same structs the OpenAI Python/JS SDKs produce on the wire.

pub use async_openai::types::chat::{
    CreateChatCompletionRequest,
    CreateChatCompletionResponse,
    ChatCompletionRequestMessage,
};
