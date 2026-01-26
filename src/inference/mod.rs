//! Inference engines for LLM text generation and embeddings.

mod embedding;
mod inference;

pub use embedding::{load_embedding_engine, EmbeddingEngine, LlamaEmbeddingEngine};
pub use inference::{load_engine, InferenceEngine, LlamaEngine};

use std::sync::Once;

/// One-time initialization for llama.cpp backend
static LLAMA_INIT: Once = Once::new();

pub(crate) fn init_llama() {
    LLAMA_INIT.call_once(|| {
        llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());
    });
}
