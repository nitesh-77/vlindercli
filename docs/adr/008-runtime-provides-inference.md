# ADR 008: Runtime Provides Inference

## Status

Superseded by ADR 018, ADR 030

*Original decision accepted, but implementation changed to queue-based message passing (ADR 018). Inference is now provided via InferenceServiceWorker listening on the `infer` queue (ADR 030).*

## Context

Agents need to reason - call an LLM to think, extract, summarize, plan. Where does inference live?

## Decision

**Runtime provides inference.** Agents call `infer(prompt)` as a host function. Runtime handles the actual LLM call (llama.cpp, ollama, etc.).

```rust
// Agent (wasm)
let clean = infer("Extract main article: ...")?;
let summary = infer("3 key takeaways: ...")?;

// Runtime provides this
fn make_infer_function() -> Function { ... }
```

## Consequences

- Agents are simpler - no LLM dependencies in wasm
- Runtime controls inference (model selection, scheduling, GPU access)
- Agent's `Model` field tells runtime which LLM to use
- Agents can call `infer` zero, one, or many times
- Agents that don't need inference just don't call it
