# ADR 053: Configurable Service Call Timeouts

**Status:** Draft

## Context

NATS pull consumers have an `inactive_threshold` — if no fetches happen within that window, the server garbage-collects the consumer. The cached client object then returns 503 forever.

This bit us in production: pensieve agent processing a long article — 23 embed calls succeeded, then ~100 seconds of infer calls for summarization, then the next embed call hung. The NATS consumer had been GC'd during the infer phase.

Interim fix (committed, not governed by this ADR):
1. Hardcoded `inactive_threshold: 300s` on all pull consumers
2. Evict and recreate on 503 — `fetch_one` detects stale consumer, evicts, retries once

The hardcoded 300s works on the machine where this was discovered (2020 MacBook Air, no GPU, ~100s infer latency). But inference latency is entirely hardware-dependent — a Raspberry Pi might take 10 minutes per infer call, a GPU box might take 2 seconds. A single hardcoded value cannot serve all hardware profiles.

Additionally, `ServiceRouter::dispatch()` has no timeout at all. If the worker process dies (not just the consumer), the dispatch loop polls forever.

## Decision

TBD. This ADR captures the problem space. Options include:

- **User-configurable timeout in fleet/agent manifest** — explicit, predictable, but adds config surface area
- **Adaptive timeout based on observed latency** — measure actual service call durations, set threshold dynamically. Risk: feels magical, hard to reason about when it goes wrong
- **No timeout, rely on 503 recovery** — the evict+recreate mechanism already handles consumer death. But doesn't help if the worker itself is dead.

Power user experience is a separate concern — exposing these knobs requires careful UX design. Deferred until first contact with real users reveals actual pain points.

## Consequences

- The interim hardcoded 300s + 503 recovery is sufficient for current workloads
- This ADR exists so the TODO comment in `nats.rs` has a home
- The dispatch timeout problem (dead worker → infinite hang) remains open
