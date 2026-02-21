//! Model catalog implementations.
//!
//! The trait is defined in the domain module.

mod ollama;
mod openrouter;

pub use ollama::OllamaCatalog;
pub use openrouter::OpenRouterCatalog;
