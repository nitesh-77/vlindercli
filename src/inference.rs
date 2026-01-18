use crate::config;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use std::num::NonZeroU32;
use std::path::Path;

pub trait InferenceEngine: Send + Sync {
    fn infer(&self, prompt: &str, max_tokens: u32) -> Result<String, String>;
}

pub struct LlamaEngine {
    backend: LlamaBackend,
    model: LlamaModel,
}

impl LlamaEngine {
    pub fn load(model_path: &Path) -> Result<Self, String> {
        let backend = LlamaBackend::init().map_err(|e| e.to_string())?;

        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .map_err(|e| e.to_string())?;

        Ok(Self { backend, model })
    }
}

impl InferenceEngine for LlamaEngine {
    fn infer(&self, prompt: &str, max_tokens: u32) -> Result<String, String> {
        let ctx_size: u32 = 8192;
        let batch_size: u32 = 2048;

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(ctx_size))
            .with_n_batch(batch_size);
        let mut ctx = self.model.new_context(&self.backend, ctx_params)
            .map_err(|e| e.to_string())?;

        // Tokenize prompt
        let mut tokens = self.model
            .str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .map_err(|e| e.to_string())?;

        // Truncate if too long (leave room for generation)
        let max_prompt_tokens = (ctx_size - max_tokens) as usize;
        if tokens.len() > max_prompt_tokens {
            tokens.truncate(max_prompt_tokens);
        }

        // Process prompt in batches
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

        // Setup sampler
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::dist(42),
            LlamaSampler::greedy(),
        ]);

        let mut output = String::new();
        let mut n_cur = tokens.len() as i32;

        // Generate tokens
        for _ in 0..max_tokens {
            let token = sampler.sample(&ctx, -1);

            // Check for end of generation
            if self.model.is_eog_token(token) {
                break;
            }

            // Convert token to string
            let token_str = self.model.token_to_str(token, llama_cpp_2::model::Special::Tokenize)
                .map_err(|e| e.to_string())?;
            output.push_str(&token_str);

            // Prepare next batch
            batch.clear();
            batch.add(token, n_cur, &[0], true).map_err(|e| e.to_string())?;
            n_cur += 1;

            ctx.decode(&mut batch).map_err(|e| e.to_string())?;
        }

        Ok(output)
    }
}

pub fn load_engine(model_name: &str) -> Result<Box<dyn InferenceEngine>, String> {
    let path = config::model_path(model_name);
    let engine = LlamaEngine::load(&path)?;
    Ok(Box::new(engine))
}
