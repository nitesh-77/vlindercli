# ADR 023: Model Manifest Format

## Status
Superseded by ADR 040 (Model Catalog) and ADR 060 (Ollama-only inference)

The manifest format is still used, but the `engine` field no longer supports `"llama"`. Supported engines are `"ollama"` and `"openrouter"`.

## Context
Agent manifests declare model requirements as names (e.g., `models = ["phi3", "nomic-embed"]`). The runtime mapped these names to files via a hardcoded convention: `~/.vlinder/models/{name}.gguf`.

This approach had problems:
- No way to know if a model is for inference or embedding
- Model locations hardcoded, not configurable per-agent
- No validation until runtime tries to use the model

## Decision
Model references in agent manifests are now **name→URI mappings** pointing to model manifest files:

```toml
# agent.toml
[requirements.models]
phi3 = "file://./models/phi3.toml"
nomic-embed = "file://./models/nomic.toml"
```

Each model manifest declares:
```toml
# models/phi3.toml
name = "phi3"
type = "inference"    # or "embedding"
engine = "llama"
model = "file://./phi3.gguf"
```

The runtime:
1. Resolves model URIs relative to agent directory
2. Loads model manifests via `loader::load_model()`
3. Validates model type matches service (inference/embedding)
4. Uses model file path from manifest

## Consequences

**Benefits:**
- Type safety: embedding models can't be used for inference
- Explicit locations: model files can live anywhere
- Per-agent models: agents can bundle their own models
- Clear errors: "model 'phi3' has type Embedding but inference was expected"

**Trade-offs:**
- More configuration: agents must declare model manifests
- Only `file://` URIs supported (future: registry://, ollama://)
- Only `llama` engine supported (future: onnx)
