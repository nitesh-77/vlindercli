# ADR 029: Harness as User Interface

## Status

Accepted

## Context

Users interact with Vlinder through different interfaces: CLI, web UI, API servers. Each interface has different needs (interactive vs batch, streaming vs request-response) but all need to run agents.

We needed a concept that separates "how users interact" from "how agents execute."

## Decision

**A Harness is the user-facing interface to the Vlinder system.**

### What a Harness Does

1. Accepts user input (commands, HTTP requests, etc.)
2. Resolves agent references to runnable configurations
3. Sends requests to runtimes
4. Presents results to users (streaming, formatted output, etc.)

### What a Harness Does NOT Do

1. Execute WASM code directly
2. Manage model loading or inference
3. Own storage or queue infrastructure

### Harness Trait

```rust
pub trait Harness {
    /// Invoke an agent. Returns immediately with a request ID.
    fn invoke(&self, agent_name: &str, input: &str) -> Result<MessageId, HarnessError>;

    /// Poll for response. Returns Some(output) if ready, None if pending.
    fn poll(&self, request_id: &MessageId) -> Result<Option<String>, HarnessError>;
}
```

The async invoke/poll pattern allows harnesses to remain responsive while agents execute via queue-based message passing.

### Implementations

| Harness | Interface | Use Case |
|---------|-----------|----------|
| `CliHarness` | Terminal | Interactive development, scripting |
| `WebHarness` | HTTP | Browser-based UI (future) |
| `ApiHarness` | REST/gRPC | Programmatic access (future) |

## Learning: Embedded Runtime

The initial `CliHarness` embedded `WasmRuntime` directly:

```rust
// Initial implementation (coupled)
pub struct CliHarness {
    runtime: WasmRuntime,  // embedded directly
}
```

This coupling was intentional — we needed to see the system work end-to-end before understanding where the boundaries should be. By building it coupled first, we learned:

1. Harness needs runtime endpoints, not runtime ownership
2. Different harnesses want the same runtime (CLI and web shouldn't duplicate)
3. Runtime lifecycle is independent of harness lifecycle

This understanding led to ADR 031, where harnesses become thin clients that query vlinderd for runtime endpoints.

## Consequences

- Clear separation between user interface and execution
- Multiple interfaces can share the same runtime infrastructure
- Harness implementation is simple — just routing and presentation
- Testing harnesses doesn't require full runtime stack
