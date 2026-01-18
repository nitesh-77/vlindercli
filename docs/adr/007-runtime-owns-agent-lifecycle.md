# ADR 007: Runtime Owns Agent Lifecycle

## Status

Accepted (updated by ADR 011)

## Context

Agents are wasm modules that need to be loaded, compiled, executed, and eventually unloaded. Wasm compilation requires an Engine (wasmtime's compilation context). Where does this live? Who manages agent lifecycle?

## Decision

**Runtime** is the kernel. It:

- Owns the Engine (shared compilation context)
- Spawns agents via `spawn_agent()`
- Manages agent lifecycle (load, execute, unload)

```rust
let runtime = Runtime::new();
let agent = runtime.spawn_agent(name, path, model, behavior)?;
agent.execute(runtime.engine(), input);
// agent dropped when done → wasm module freed
```

Agent::load() exists but is internal. Public interface is Runtime.

## Consequences

- Single Engine shared across all agents (efficient)
- Runtime is the entry point for all agent operations
- Future: Runtime will handle scheduling, capabilities, orchestration
- Clear ownership: Runtime → Agent → Module
