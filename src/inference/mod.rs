//! Inference engine implementations.
//!
//! The trait is defined in the domain module.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;

use crate::domain::{InferenceEngine, Model};

// Shared backend - initialized once, used by both inference and embedding
static LLAMA_BACKEND: OnceLock<LlamaBackend> = OnceLock::new();

/// Get the shared llama backend, initializing it if necessary.
pub fn get_backend() -> Result<&'static LlamaBackend, String> {
    Ok(LLAMA_BACKEND.get_or_init(|| {
        llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());
        LlamaBackend::init().expect("Failed to initialize llama backend")
    }))
}

/// Open an inference engine for the given model.
pub fn open_inference_engine(model: &Model) -> Result<Arc<dyn InferenceEngine>, String> {
    let path = model.model_path.path()
        .ok_or_else(|| format!("unsupported model path: {}", model.model_path))?;

    let engine = LlamaEngine::load(Path::new(path))?;
    Ok(Arc::new(engine))
}

// ============================================================================
// In-Memory Implementation (for testing)
// ============================================================================

/// In-memory inference engine that returns canned responses.
pub struct InMemoryInference {
    response: String,
}

impl InMemoryInference {
    pub fn new(response: impl Into<String>) -> Self {
        Self { response: response.into() }
    }
}

impl InferenceEngine for InMemoryInference {
    fn infer(&self, _prompt: &str, _max_tokens: u32) -> Result<String, String> {
        Ok(self.response.clone())
    }
}

// ============================================================================
// Llama Implementation
// ============================================================================

pub struct LlamaEngine {
    model: LlamaModel,
}

impl LlamaEngine {
    pub fn load(model_path: &Path) -> Result<Self, String> {
        let backend = get_backend()?;

        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(backend, model_path, &model_params)
            .map_err(|e| e.to_string())?;

        Ok(Self { model })
    }
}

impl InferenceEngine for LlamaEngine {
    fn infer(&self, prompt: &str, max_tokens: u32) -> Result<String, String> {
        let backend = get_backend()?;
        let ctx_size: u32 = 8192;
        let batch_size: u32 = 2048;

        let formatted_prompt = format!("<|user|>\n{}<|end|>\n<|assistant|>\n", prompt);

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(ctx_size))
            .with_n_batch(batch_size);
        let mut ctx = self.model.new_context(backend, ctx_params)
            .map_err(|e| e.to_string())?;

        let mut tokens = self.model
            .str_to_token(&formatted_prompt, llama_cpp_2::model::AddBos::Always)
            .map_err(|e| e.to_string())?;

        let max_prompt_tokens = (ctx_size - max_tokens) as usize;
        if tokens.len() > max_prompt_tokens {
            tokens.truncate(max_prompt_tokens);
        }

        let mut batch = LlamaBatch::new(batch_size as usize, 1);
        let mut i = 0;
        while i < tokens.len() {
            batch.clear();
            let chunk_end = (i + batch_size as usize).min(tokens.len());
            for j in i..chunk_end {
                let is_last = j == tokens.len() - 1;
                batch.add(tokens[j], j as i32, &[0], is_last).map_err(|e| e.to_string())?;
            }
            ctx.decode(&mut batch).map_err(|e| e.to_string())?;
            i = chunk_end;
        }

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::dist(42),
            LlamaSampler::greedy(),
        ]);

        let mut output = String::new();
        let mut n_cur = tokens.len() as i32;

        for _ in 0..max_tokens {
            let token = sampler.sample(&ctx, -1);

            if self.model.is_eog_token(token) {
                break;
            }

            let token_str = self.model.token_to_str(token, llama_cpp_2::model::Special::Tokenize)
                .map_err(|e| e.to_string())?;
            output.push_str(&token_str);

            batch.clear();
            batch.add(token, n_cur, &[0], true).map_err(|e| e.to_string())?;
            n_cur += 1;

            ctx.decode(&mut batch).map_err(|e| e.to_string())?;
        }

        Ok(output)
    }
}

