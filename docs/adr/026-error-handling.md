# ADR 026: Error Handling

## Status

Superseded — assumes WASM-era cross-agent SDK calls that no longer exist (ADR 046, 075)

## Context

ADR 019 establishes that cross-agent calls require explicit timeouts and return protocol-level errors. This ADR defines the complete error handling philosophy: what errors exist, how they propagate, and who handles retries.

## Decision

### 1. Typed Error Taxonomy

Errors are typed at the protocol level. The taxonomy will grow as the system matures, but the core categories are:

```rust
pub enum SdkError {
    /// Call to another agent timed out (ADR 019)
    Timeout {
        agent: String,
        timeout: Duration,
    },

    /// Target agent returned an error
    AgentError {
        agent: String,
        message: String,
    },

    /// Queue rejected message (backpressure)
    QueueFull {
        queue: String,
    },

    /// Request or response malformed
    InvalidMessage {
        reason: String,
    },

    /// Called an agent not declared in manifest (ADR 021)
    UndeclaredDependency {
        agent: String,
    },

    /// Infrastructure failure (network, storage, etc.)
    Infrastructure {
        message: String,
    },
}
```

**Rationale:**
- Typed errors are self-documenting
- Each variant is actionable—caller knows what went wrong
- Taxonomy grows incrementally; we don't need to enumerate everything now

### 2. No Auto-Retry at SDK Level

The SDK does not automatically retry failed calls. Errors are returned immediately to the caller.

```rust
// Caller sees error immediately, decides what to do
match sdk.call_agent("summarizer", input, Timeout::Seconds(30)) {
    Ok(response) => process(response),
    Err(SdkError::Timeout { .. }) => {
        // Caller decides: retry? fail? use fallback?
    }
    Err(e) => return Err(e),
}
```

**Rationale:**
- Explicit timeouts (ADR 019) already force callers to think about failure
- Retry policy is domain-specific—SDK can't know the right choice
- Hidden retries obscure latency and make debugging harder
- Infrastructure (queue redelivery) can handle retries if configured

### 3. Wrapped Error Propagation

When agent A calls agent B and B fails, A receives a wrapped error identifying B.

```rust
// Agent A calls agent B
let result = sdk.call_agent("B", input, Timeout::Seconds(30));

// If B fails, A sees:
Err(SdkError::AgentError {
    agent: "B".to_string(),
    message: "B's error message".to_string(),
})
```

**Rationale:**
- Orchestrators need to know which agent failed for subsequent planning
- Enables intelligent error handling (retry B? skip B? use alternative?)
- Doesn't expose deep internals—just the immediate failure point

**Not included:**
- Full call stack / error chain (complexity without clear value)
- Structured inner error (agent's error is opaque to caller)

## Consequences

- **Explicit error handling:** Callers must handle errors; no silent failures
- **Debuggable:** Error type tells you what failed and where
- **Caller autonomy:** Retry policy is caller's decision, not SDK's
- **Incremental taxonomy:** New error variants added as needed

## Open Questions

- Should `AgentError` include a structured error code in addition to message?
- Error logging/tracing story (correlation IDs, spans)
- Dead letter queue behavior for unrecoverable errors
