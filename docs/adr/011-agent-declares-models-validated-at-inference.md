# ADR 011: Agent Declares Models, Validated at Inference

## Status

Accepted (updates ADR 007, 008, 010; path conventions updated by ADR 012)

## Context

Previous ADRs established:
- ADR 007: Runtime owns lifecycle via `spawn_agent()`
- ADR 008: Agent calls `infer(prompt)`, model field tells runtime which LLM
- ADR 009: ModelType enum determines loading (superseded by 010)
- ADR 010: Model identified by name, has model_type field

Through iteration, we discovered:
1. `spawn_agent()` added indirection without value — `Agent::load()` is sufficient
2. Single `infer(prompt)` assumes one model — agents may need different models for different tasks
3. `ModelType` was redundant — model name is sufficient for routing
4. Declaration and runtime call are different concerns (like package.json vs require)

## Decision

**Agent declares models upfront. Runtime validates at inference time.**

```rust
pub struct Model {
    pub name: String,  // "phi3", "llama3", "nomic-embed"
}

pub struct Agent {
    pub name: String,
    pub models: Vec<Model>,  // declaration: what this agent needs
    wasm_path: String,
}

// Load agent with its model declarations
let agent = Agent::load("reader-agent", vec![
    Model { name: "phi3".to_string() },
]).unwrap();

// Runtime executes agent, validates infer() calls
let result = runtime.execute(&agent, input);
```

Agent wasm calls `infer(model_name, prompt)`:
```rust
// Inside wasm - agent chooses which declared model to use
let extracted = infer("phi3".to_string(), "Extract: ...".to_string());
let summary = infer("phi3".to_string(), "Summarize: ...".to_string());
```

Runtime validates before executing:
```rust
if !agent.has_model(&model_name) {
    return format!("[error] model '{}' not declared by agent", model_name);
}
// Only then: load_engine(&model_name).infer(&prompt)
```

**Convention-based paths:**
- Wasm: `agent_wasm/{name_with_underscores}.wasm`
- Models: `models/{name}.gguf`

## Consequences

- Agent identity comes from Rust (trusted), model requests from wasm (validated)
- Same agent can use multiple models for different reasoning steps
- No `spawn_agent()` needed — `Agent::load()` + `runtime.execute()` is sufficient
- No `ModelType` needed — runtime infers loader from model name/extension
- Convention over configuration for paths
- Future: different loading parameters (temperature, system prompt) passed per infer() call
