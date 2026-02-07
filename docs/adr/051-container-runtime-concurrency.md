# ADR 051: Container Runtime Concurrency

**Status:** Proposed

## Problem

Two levels of serialization constrain container runtime throughput.

### Cross-agent serialization

`ContainerRuntime` uses a single `running: Option<RunningTask>` slot. While a task is in flight, `tick()` returns early without checking for new work. This serializes all container agent invocations — not just per-agent, but across all agents.

If Agent A is processing a 10-second invocation, Agent B's queued work waits even though the two are completely independent containers.

In distributed mode this is partially mitigated by running multiple worker processes, each with its own `ContainerRuntime`. But within a single process, all container agents share one slot.

### Per-agent serialization

Even with cross-agent parallelism solved, each agent processes one invocation at a time. If multiple customers invoke the same agent concurrently, requests queue up behind each other. The container is a long-running process that could handle concurrent HTTP requests, but the runtime only dispatches one at a time.

Scaling options with different architectural implications:

- **Horizontal** — multiple container replicas per agent, each sequential. Scale by adding instances.
- **Vertical** — single container handles concurrent `/invoke` requests. Requires per-invocation isolation in the bridge (each concurrent SDK call context needs its own submission ID routing).
- **Sequential** — one at a time per agent, queue absorbs bursts. Simple, predictable, debuggable.

## The product question

Before choosing a scaling strategy, we need to decide what an agent *is*:

- **Function** — stateless, many copies, each invocation independent. Scaling is trivial (more replicas). Agent authors assume nothing about ordering or shared state.
- **Process** — one instance, sequential processing, has identity and continuity. Simple mental model. Agent authors can reason about ordering. Scaling requires explicit replica configuration.
- **Service** — one instance, concurrent requests, manages its own concurrency. Most flexible, most complex for agent authors. SDK must provide isolation guarantees.

This is a product decision that shapes the developer contract: what the SDK promises, how agents are written, what "deploy" means, and what users expect about state and ordering. The technical scaling strategy follows from it.

## Impact

- Independent agents block each other
- Same-agent concurrent requests serialize behind each other
- Throughput scales with worker processes, not with deployed agents or concurrent users
- Local (single-process) mode is hardest hit — no parallelism at all
