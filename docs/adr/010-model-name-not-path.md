# ADR 010: Model Name, Not Path

## Status

Accepted (supersedes ADR 009, updated by ADR 011)

## Context

ADR 009 established that `Model.path` serves as both identifier and location. However, this conflicts with our abstraction levels:

```
Agent builders think in: model names, system prompts, temperatures.
Agent builders don't think about: Rust crates, C++ bindings, GGUF formats.
```

Requiring agent builders to know filesystem paths (`models/phi3.gguf`) leaks infrastructure concerns into the agent layer.

## Decision

**Model is identified by name, not path.** Runtime resolves names to paths.

```rust
pub struct Model {
    pub model_type: ModelType,
    pub name: String,  // "phi3", "llama3", "nomic-embed"
}
```

Agent builders write:
```rust
Model { model_type: ModelType::Inference, name: "phi3".to_string() }
```

Runtime maintains a registry that resolves `"phi3"` to the appropriate path and loader.

## Consequences

- Agent builders work at the right abstraction level (names, not paths)
- Runtime owns the resolution layer (name → path → loader)
- Same agent definition works across machines with different filesystem layouts
- Registry becomes a new runtime responsibility (config file or convention-based)
