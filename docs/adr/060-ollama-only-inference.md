# ADR 060: Drop llama.cpp and WASM, Standardize on Ollama + OpenRouter

## Status

Accepted

Supersedes ADR 023 (Model Manifest Format) and ADR 040 (Model Catalog) for engine-related decisions.

## Context

Vlinder originally embedded two heavyweight runtimes:

1. **WASM via extism** — agents were compiled to WebAssembly and executed in-process. ADR 046 replaced this with OCI containers via Podman, making WASM a language choice rather than a platform requirement. The extism dependency was removed.

2. **llama.cpp via llama-cpp-2** — local inference and embedding using GGUF model files. This pulled in native build dependencies (CMake, Metal framework on macOS, bindgen) that made builds slow and fragile. Every `cargo build` compiled C++ from source.

Meanwhile, Ollama provides the same local inference capability (it uses llama.cpp internally) with none of the build cost. OpenRouter provides cloud inference via an OpenAI-compatible API. Together they cover all inference and embedding needs.

### Why llama.cpp no longer makes sense

- **Build overhead**: llama-cpp-2 requires CMake, a C++ compiler, and platform-specific frameworks (Metal on macOS). This adds minutes to clean builds and breaks on CI without careful setup.
- **Redundant with Ollama**: Ollama wraps llama.cpp behind an HTTP API, handles model management, quantization, and GPU dispatch. Vlinder was reimplementing what Ollama already provides.
- **Maintenance burden**: `LlamaEngine`, `LlamaEmbeddingEngine`, `LLAMA_BACKEND` static initialization, and `get_backend()` were ~150 lines of unsafe-adjacent code for token management, batching, and sampling — all handled by Ollama automatically.
- **No users on llama engine**: All deployed agents use Ollama or OpenRouter models.

## Decision

**Remove llama-cpp-2 and the `EngineType::Llama` variant.** All local inference goes through Ollama. Cloud inference goes through OpenRouter.

### Supported engines

| Engine | Use case | Transport |
|--------|----------|-----------|
| `Ollama` | Local inference and embedding | HTTP API (`/api/generate`, `/api/embed`) |
| `OpenRouter` | Cloud inference (Claude, GPT, etc.) | OpenAI-compatible REST API |
| `InMemory` | Testing | In-process |

### What was removed

- `llama-cpp-2` crate dependency
- `EngineType::Llama` enum variant
- `ModelEngineConfig::Llama` manifest variant
- `LlamaEngine` and `LlamaEmbeddingEngine` structs
- `LLAMA_BACKEND` static and `get_backend()` initialization
- `InferenceKind::Llama(LlamaConfig)` and `LlamaConfig` struct
- `llama_level` logging configuration
- All `"llama"` match arms in serialization/dispatch

### Wire compatibility

The protobuf `ENGINE_TYPE_LLAMA = 1` value is retained but deprecated. Inbound messages with this value map to `EngineType::Ollama` so existing persisted data doesn't break.

## Consequences

**Positive:**
- Clean builds are significantly faster (no C++ compilation)
- Simpler dependency tree — no native build toolchain required
- Ollama handles model management, GPU dispatch, and quantization
- Fewer engine variants to maintain across domain types, serialization, and dispatch

**Negative:**
- Ollama must be running for local inference (was already true for Ollama-engine models)
- No direct GGUF file loading — models must be imported into Ollama first
