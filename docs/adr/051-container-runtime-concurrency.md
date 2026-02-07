# ADR 051: Cross-Agent Parallelism in Container Runtime

**Status:** Accepted

## Context

`ContainerRuntime` uses a single `running: Option<RunningTask>` slot. While a task is in flight, `tick()` returns early without checking for new work. This serializes all container agent invocations — not just per-agent, but across all agents.

If Agent A is processing a 10-second invocation, Agent B's queued work waits even though the two are completely independent containers.

In distributed mode this is partially mitigated by running multiple worker processes, each with its own `ContainerRuntime`. But within a single process, all container agents share one slot. Local mode is hardest hit — no parallelism at all.

## Decision

Replace the single `running: Option<RunningTask>` slot with `running: HashMap<String, RunningTask>` keyed by agent name. `tick()` checks all running tasks for completion and can dispatch new work to any agent that isn't already busy.

Each agent still processes one invocation at a time (sequential per-agent). Per-agent concurrency is a separate product decision (future ADR).

## Consequences

- Independent agents no longer block each other
- Local mode throughput scales with the number of deployed agents, not just worker processes
- Per-agent concurrency is unchanged (still sequential) — whether that's the right model is a separate decision
