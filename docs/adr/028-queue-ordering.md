# ADR 028: Queue Ordering Guarantees

## Status

Accepted (validated in 38d908e)

## Context

ADR 018 establishes a queue-based message-passing architecture. A common question in distributed systems: does the queue guarantee message ordering?

Ordering guarantees are expensive—they limit parallelism and add coordination overhead. Most agent work is naturally parallel (fan-out to chunks, parallel embedding, independent operations).

## Decision

### 1. No Ordering Guarantees

The queue does not guarantee message ordering. Messages may be processed in any order.

```
Producer sends: [A, B, C]
Consumer may see: [B, A, C] or [C, A, B] or any permutation
```

**Rationale:**
- Enables maximum parallelism
- Matches reality of distributed queues (Kafka partitions, SQS, etc.)
- Most operations don't need ordering
- Ordering is expensive to guarantee

### 2. Orchestrator Manages Sequencing

When ordering matters, the orchestrator manages it. This is the saga pattern: a coordinator that sequences operations and handles intermediate results.

```rust
// Orchestrator sequences the workflow
let chunks = sdk.call_agent("chunker", document, Timeout::Seconds(60))?;

// Fan out - parallel, order doesn't matter
let summaries: Vec<_> = chunks
    .par_iter()
    .map(|chunk| sdk.call_agent("summarizer", chunk, Timeout::Seconds(30)))
    .collect()?;

// Reduce - depends on summaries completing first
let final_summary = sdk.call_agent("reducer", summaries, Timeout::Seconds(30))?;
```

**Rationale:**
- Workflows aren't static—orchestrators make dynamic decisions based on intermediate results
- Orchestrator has the context to know what depends on what
- No magic ordering assumptions in the queue layer
- Explicit sequencing is debuggable

**Not supported:**
- Queue-level FIFO guarantees
- Automatic correlation ID ordering
- Implicit workflow sequencing

### 3. Parallel by Default

The system assumes operations are independent unless the orchestrator explicitly sequences them.

| Pattern | Ordering | How |
|---------|----------|-----|
| Fan-out | Parallel | Multiple `call_agent` in parallel |
| Map-reduce | Fan-out parallel, reduce sequential | Orchestrator awaits all, then reduces |
| Pipeline | Sequential | Orchestrator chains calls |
| Dynamic workflow | Mixed | Orchestrator decides at runtime |

## Consequences

- **Maximum parallelism:** No artificial ordering constraints
- **Explicit sequencing:** Orchestrator controls workflow, not queue
- **Debuggable:** Sequence is visible in orchestrator code
- **Flexible:** Dynamic workflows based on intermediate results

## Open Questions

- Should SDK provide helpers for common patterns (fan-out, map-reduce)?
- Correlation IDs for tracing related messages (observability, not ordering)
