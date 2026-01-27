//! Embedding engine implementations.
//!
//! The trait is defined in the domain module.

pub mod dispatch;

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Arc, Once};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;

use crate::domain::{EmbeddingEngine, Model};

static LLAMA_INIT: Once = Once::new();

fn init_llama() {
    LLAMA_INIT.call_once(|| {
        llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());
    });
}

/// Open an embedding engine for the given model.
pub fn open_embedding_engine(model: &Model) -> Result<Arc<dyn EmbeddingEngine>, String> {
    let model_path = model.model.strip_prefix("file://")
        .ok_or_else(|| format!("unsupported model URI scheme: {}", model.model))?;

    let engine = LlamaEmbeddingEngine::load(Path::new(model_path))?;
    Ok(Arc::new(engine))
}

// ============================================================================
// In-Memory Implementation (for testing)
// ============================================================================

/// In-memory embedding engine that returns canned responses.
pub struct InMemoryEmbedding {
    embedding: Vec<f32>,
}

impl InMemoryEmbedding {
    pub fn new(embedding: Vec<f32>) -> Self {
        Self { embedding }
    }
}

impl EmbeddingEngine for InMemoryEmbedding {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, String> {
        Ok(self.embedding.clone())
    }
}

// ============================================================================
// Llama Implementation
// ============================================================================

pub struct LlamaEmbeddingEngine {
    backend: LlamaBackend,
    model: LlamaModel,
}

impl LlamaEmbeddingEngine {
    pub fn load(model_path: &Path) -> Result<Self, String> {
        init_llama();
        let backend = LlamaBackend::init().map_err(|e| e.to_string())?;

        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .map_err(|e| e.to_string())?;

        Ok(Self { backend, model })
    }
}

impl EmbeddingEngine for LlamaEmbeddingEngine {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let ctx_size: u32 = 2048;
        let batch_size: u32 = 512;

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(ctx_size))
            .with_n_batch(batch_size)
            .with_embeddings(true);

        let mut ctx = self.model.new_context(&self.backend, ctx_params)
            .map_err(|e| e.to_string())?;

        let tokens = self.model
            .str_to_token(text, llama_cpp_2::model::AddBos::Always)
            .map_err(|e| e.to_string())?;

        let mut batch = LlamaBatch::new(batch_size as usize, 1);
        for (i, &token) in tokens.iter().enumerate() {
            let is_last = i == tokens.len() - 1;
            batch.add(token, i as i32, &[0], is_last).map_err(|e| e.to_string())?;
        }

        ctx.decode(&mut batch).map_err(|e| e.to_string())?;

        let embeddings = ctx.embeddings_seq_ith(0)
            .map_err(|e| format!("failed to get embeddings: {}", e))?;

        Ok(embeddings.to_vec())
    }
}

