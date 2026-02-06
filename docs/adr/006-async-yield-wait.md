# ADR 006: Async Yield with Wait

## Status

Superseded by ADR 046

WASM yield/wait was never implemented. Agents are now OCI containers — async parallelism is a language-level concern, not a platform one.

## Context

Agents may need to issue multiple operations in parallel (independent branches of a DAG). Synchronous `yield_to` blocks on each call - sequential execution only.

Runtime owns scheduling. If agent can issue multiple yields before waiting, runtime can schedule them in parallel when hardware permits.

## Domain Decision

Async yield is the primitive. Sync yield is sugar.

```
yield_to_async(agent, input) -> handle   // non-blocking, returns immediately
wait(handle) -> response                  // blocks until result ready
```

Agent issues multiple yields, continues executing, then waits for results:

```
h1 = yield_to_async(agent_a, input_a)
h2 = yield_to_async(agent_b, input_b)
// agent continues, runtime can schedule both
r1 = wait(h1)
r2 = wait(h2)
```

Sync yield is just:
```
yield_to(agent, input) = wait(yield_to_async(agent, input))
```

## Consequences

- Runtime can schedule parallel operations when resources allow
- Agent controls when to block (flexibility)
- DAG execution becomes possible
- Wire protocol: `yield_to_async` returns handle, `wait` takes handle
