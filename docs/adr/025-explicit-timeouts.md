# ADR 025: Explicit Timeouts in SDK

## Status

Draft

## Context

ADR 018 establishes a queue-based, message-passing architecture where agents communicate via typed messages. The SDK provides convenience methods like `sdk.call_agent()` that look synchronous but cross process/network boundaries.

The danger: making distributed calls look like local calls. Without explicit timeouts, callers can block indefinitely. The Erlang philosophy is instructive—Erlang makes message passing explicit and requires timeouts on receives. We should do the same.

## Decision

### 1. Timeouts Are Required

All cross-agent calls require an explicit timeout parameter.

```rust
// Rust SDK
sdk.call_agent("summarizer", input, Timeout::Seconds(30))?;
```

**Rationale:**
- Forces developers to think about latency
- No hidden infinite waits
- Makes the distributed nature explicit
- Verbose is intentional—distributed calls *should* feel different

### 2. Protocol-Level Errors

The protocol defines error kinds. Each SDK translates to language-idiomatic representation.

**Protocol message:**

```rust
enum Response {
    Ok { payload: Vec<u8> },
    Error { kind: ErrorKind, message: String },
}

enum ErrorKind {
    Timeout,
    AgentCrashed,
    QueueFull,
    InvalidMessage,
    Infrastructure,
}
```

**SDK representations:**

| Language | Idiom |
|----------|-------|
| Rust | `Result<AgentResponse, SdkError>` with `?` operator |
| Python | Returns value or `raise TimeoutError` |
| Go | `(response, error)` tuple |
| JavaScript | Promise rejection |

**Key principle:** No silent failures. Whatever the language idiom, timeout must be surfaced explicitly—not swallowed, not retried silently.

### 3. Per-Call Scope

Each `call_agent` invocation has its own timeout. No cascade timeouts (where remaining time propagates to nested calls).

```rust
// Each call manages its own timeout
let summary = sdk.call_agent("summarizer", text, Timeout::Seconds(30))?;
let embedding = sdk.call_agent("embedder", summary, Timeout::Seconds(10))?;
```

**Rationale:**
- Simplest mental model
- Cascade timeouts add complexity with unclear value
- Can revisit if real use cases demand it

## Consequences

- **Explicit latency awareness:** Developers must consider timeout for every cross-agent call
- **Consistent error handling:** Protocol defines errors once, SDKs translate idiomatically
- **No surprise hangs:** Every distributed operation has a bounded wait time
- **Slightly more verbose:** Intentional trade-off for honesty about distributed nature

## Open Questions

- Exact `Timeout` type API (enum variants? builder methods?)
- Whether inference/embedding calls also need explicit timeouts (they're also expensive operations)
- Retry semantics (covered in separate ADR)
