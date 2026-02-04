# ADR 040: Model Catalog

## Status

Accepted

## Context

Models currently come from a hardcoded directory scanned at daemon init. This has problems:

1. **No discoverability** — users must manually place GGUF files in the right location
2. **Single source** — only local files, no Ollama/HuggingFace integration
3. **Implicit registration** — models appear "magically" based on filesystem state

We want users to explicitly add models from various sources:

```bash
vlinder model add llama3 --catalog ollama
vlinder model add nomic-embed-text --catalog huggingface
```

### The Coupling Problem

Model catalogs (where models come from) and inference engines (how models run) are coupled:

- Model from Ollama → must run via Ollama HTTP API
- Model from HuggingFace (GGUF) → can run via llama.cpp locally

This coupling is not a problem to avoid — it should be explicit in the domain model.

## Decision

### 1. ModelCatalog Trait

A catalog resolves model names to manifests. The manifest's `engine` field encodes how to run it.

```
trait ModelCatalog
    resolve(name) → Result<Model>
    list() → Vec<ModelInfo>
    available(name) → bool
```

**Source**: `src/domain/catalog.rs`

### 2. Catalog Implementations

| Implementation | Source | Produces | Engine |
|----------------|--------|----------|--------|
| `OllamaCatalog` | Ollama API (`/api/tags`) | Model with `engine: Ollama` | `OllamaInferenceEngine` |
| `HuggingFaceCatalog` | HF Hub, downloads GGUF | Model with `engine: Llama` | `LlamaEngine` |
| `LocalCatalog` | Scans local directory | Model with `engine: Llama` | `LlamaEngine` |

### 3. Ollama Engine

New engine type for models served by Ollama:

```
OllamaInferenceEngine
    endpoint: "http://localhost:11434"
    model: "llama3:8b"

    infer(prompt, max_tokens) → HTTP POST /api/generate
```

Similarly `OllamaEmbeddingEngine` for embeddings.

### 4. Engine Dispatch

Extend existing dispatch to route by engine type:

```
EngineType::Llama  → LlamaEngine (existing)
EngineType::Ollama → OllamaInferenceEngine (new)
EngineType::OpenAI → OpenAIInferenceEngine (future)
```

### 5. Model Commands

```bash
vlinder model add <name> --catalog <ollama|huggingface|local>
vlinder model list
vlinder model remove <name>
vlinder model info <name>
```

### 6. Resolution Flow

```
vlinder model add llama3 --catalog ollama
    │
    ▼
OllamaCatalog.resolve("llama3")
    │
    ├── Queries Ollama API
    ├── Confirms model exists
    └── Returns Model {
            name: "llama3",
            engine: Ollama,
            model_path: "ollama://llama3:latest"
        }
    │
    ▼
Registry.register_model(model)
```

At agent runtime:

```
Agent requests inference with model "llama3"
    │
    ▼
Registry.get_model("llama3") → Model { engine: Ollama, ... }
    │
    ▼
Dispatch by engine type → OllamaInferenceEngine
    │
    ▼
HTTP POST to Ollama server
```

### 7. Registry Persistence

Models are persisted via `RegistryRepository` trait with SQLite as the default implementation:

```
trait RegistryRepository
    save_model(model) → Result<()>
    load_models() → Result<Vec<Model>>
    delete_model(name) → Result<bool>
    model_exists(name) → Result<bool>
```

**Source**: `src/domain/registry_repository.rs` (trait), `src/storage/registry.rs` (SQLite implementation)

**Storage location**: `.vlinder/registry.db`

The Daemon loads models from SQLite on startup. CLI commands persist models via the repository.

### 8. Remove Hardcoded Models

Delete the vlinder models directory and daemon init hardcoding. Models only exist if explicitly added via `vlinder model add`.


## Consequences

**Positive:**
- Explicit model management — users control what's available
- Multiple sources — Ollama, HuggingFace, local files
- Coupling is explicit — manifest encodes how to run the model
- Follows existing patterns — traits, dispatch, registry

**Negative:**
- Extra step for users — must `model add` before using
- Ollama dependency for Ollama models — but this is inherent, not new

## Open Questions

- Should we auto-detect running Ollama and list available models?
- Default catalog if `--catalog` not specified?
- How to handle model updates (re-pull from catalog)?
