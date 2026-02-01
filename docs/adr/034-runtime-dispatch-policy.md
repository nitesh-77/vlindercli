# ADR 034: Runtime Dispatch Policy

## Status

Accepted

## Context

ADR 031 established vlinderd as the runtime registry. The Registry tracks available runtimes and agents. When an agent is registered, we need to determine which runtime will execute it.

Options considered:
1. **Explicit declaration** - Agent manifest declares its runtime type
2. **Pattern matching** - Infer runtime from agent.id URI pattern
3. **Capability probing** - Ask each runtime if it can handle the agent

## Decision

**Runtime selection is pattern-based on `agent.id`.**

The Registry selects the runtime by matching the agent's ResourceId against known patterns:

| Pattern | RuntimeType |
|---------|-------------|
| `file://` scheme + `.wasm` extension | Wasm |

Selection logic in `Registry::select_runtime()`:

```rust
pub fn select_runtime(&self, agent: &Agent) -> Option<RuntimeType> {
    if agent.id.scheme() == Some("file") {
        if let Some(path) = agent.id.path() {
            if path.ends_with(".wasm") && self.available_runtimes.contains(&RuntimeType::Wasm) {
                return Some(RuntimeType::Wasm);
            }
        }
    }
    None
}
```

**Fail fast**: If no runtime matches, agent registration fails immediately.

## Consequences

- **Simple**: No new manifest fields needed
- **Predictable**: URI pattern determines runtime unambiguously
- **Extensible**: New patterns added as runtimes are supported:
  - `arn:aws:lambda:*` → Lambda
  - `docker://` or OCI reference → Container
- **Restrictive**: Only `file://` supported initially (local development focus)
- **Compile-time**: RuntimeType enum changes require code changes

## Future Patterns

When new runtimes are added:

```rust
// Lambda
if agent.id.as_str().starts_with("arn:aws:lambda:") {
    return Some(RuntimeType::Lambda);
}

// Container
if agent.id.scheme() == Some("docker") {
    return Some(RuntimeType::Container);
}
```
