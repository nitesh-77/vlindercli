//! Embedding engine for vector representations.

use std::num::NonZeroU32;
use std::path::Path;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;

use crate::config;

use super::init_llama;

pub trait EmbeddingEngine: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}

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

        // Tokenize text
        let tokens = self.model
            .str_to_token(text, llama_cpp_2::model::AddBos::Always)
            .map_err(|e| e.to_string())?;

        // Process in a single batch
        let mut batch = LlamaBatch::new(batch_size as usize, 1);
        for (i, &token) in tokens.iter().enumerate() {
            let is_last = i == tokens.len() - 1;
            batch.add(token, i as i32, &[0], is_last).map_err(|e| e.to_string())?;
        }

        ctx.decode(&mut batch).map_err(|e| e.to_string())?;

        // Get embeddings from sequence
        let embeddings = ctx.embeddings_seq_ith(0)
            .map_err(|e| format!("failed to get embeddings: {}", e))?;

        Ok(embeddings.to_vec())
    }
}

pub fn load_embedding_engine(model_name: &str) -> Result<Box<dyn EmbeddingEngine>, String> {
    let path = config::model_path(model_name);
    let engine = LlamaEmbeddingEngine::load(&path)?;
    Ok(Box::new(engine))
}
