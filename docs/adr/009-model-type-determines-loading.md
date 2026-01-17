# ADR 009: Model Type Determines Loading

## Status

Accepted

## Context

Agents may need multiple models of different types: inference (LLM), embedding, speech-to-text, vision, etc. Each type requires different loading behavior and backend.

## Decision

**Model type determines loading behavior.** Each model has a type and a path. Runtime dispatches to the appropriate loader based on type.

```rust
pub enum ModelType {
    Inference,
    Embedding,
}

pub struct Model {
    pub model_type: ModelType,
    pub path: String,
}

pub struct Agent {
    pub models: Vec<Model>,
    // ...
}
```

Agent declares multiple models. Runtime validates at load time. Path is the unique identifier.

## Consequences

- Agent can declare all models it needs (inference + embedding, etc.)
- Runtime knows how to load each type (llama.cpp for inference, etc.)
- Path is both identifier and location - no separate resolution layer
- Validation happens at spawn time, not runtime
