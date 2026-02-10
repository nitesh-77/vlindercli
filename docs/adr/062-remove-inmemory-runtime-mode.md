# ADR 062: Remove In-Memory Runtime Mode

## Status

Draft

## Context

Vlinder has two runtime modes:

1. **Distributed** — NATS queue, gRPC registry, supervisor-spawned worker processes. This is the production architecture.
2. **In-memory** — Single-process, in-memory queue, no NATS. Used for `vlinder agent run` without infrastructure and for unit tests.

CLAUDE.md already calls in-memory mode "vestigial — exists purely to facilitate linear debugging." But it's still the default (`queue.backend = "memory"`), and it shapes how new features get designed.

### The problem

In-memory mode is a gravity well for bad design. When a capability works in single-process with an in-memory queue, there's no forcing function to make the distributed design correct. Two concrete examples:

- **DAG capture (ADR 061 spike):** The spike bolted DAG capture onto `CliHarness` as struct fields — stash payload on invoke, capture node on tick. This worked perfectly in-memory. In distributed mode it's fundamentally broken: the VLINDER stream uses WorkQueue retention, so a DAG worker can't observe the same messages the runtime and harness consume. The entire design had to be discarded.

- **Every new worker capability** pays a tax: design it for in-memory (easy), discover it doesn't translate to distributed, redesign as a queue worker, then maintain both paths or accept that in-memory is incomplete.

The in-memory queue also diverges from NATS semantics in ways that hide bugs: no consumer groups, no retention policy, no ack deadlines, no subject-based routing. Tests passing against InMemoryQueue doesn't mean the feature works on NATS.

### What in-memory mode provides today

- `vlinder agent run` / `vlinder fleet run` without NATS running
- Fast unit tests with no infrastructure dependency
- Single-thread debugging

### What it costs

- Two code paths for queue, storage, and registry (InMemoryQueue, InMemoryRegistry, StorageObjectMemory, StorageVectorMemory)
- `from_config()` branches on backend, `run_local()` vs `run_distributed()` in every command
- `Daemon::new()` wires in-memory by default — the wrong default
- InMemoryQueue semantics don't match NATS, masking real bugs
- Designs gravitate toward in-memory-first, producing patterns that break distributed

## Decision

**Remove in-memory as a runtime mode.** All runtime execution goes through NATS. In-memory implementations survive only as test doubles.

### What changes

- `queue.backend = "memory"` config option removed. NATS is the only queue backend.
- `from_config()` always returns `NatsQueue`.
- `Daemon::new()` connects to NATS.
- `run_local()` / `run_distributed()` collapse into a single path in agent.rs and fleet.rs. Local mode starts a local NATS (or requires one running).
- `StorageObjectMemory` and `StorageVectorMemory` worker roles removed from supervisor/config. SQLite backends are sufficient.
- `InMemoryQueue`, `InMemoryRegistry`, and in-memory storage implementations stay in the codebase but become `#[cfg(test)]`-only or explicit test fixtures.

### What doesn't change

- Unit tests continue to use `InMemoryQueue` and `InMemoryRegistry` as test doubles — they're fast and deterministic for testing domain logic.
- The `MessageQueue` trait stays. The abstraction is sound; the problem is having two production implementations.
- NATS remains a local dependency. `vlinder daemon` starts it or expects it running.

## Consequences

**Positive:**
- One code path. Every feature is distributed-first by construction.
- New capabilities are workers from day one — no in-memory shortcut to design around.
- NATS semantics (consumer groups, retention, ack deadlines) are always exercised.
- Less code to maintain: remove `run_local()` variants, `StorageObjectMemory`, `StorageVectorMemory` worker roles, `queue.backend` config.

**Negative:**
- NATS must be running for any `vlinder` command that touches the queue. Local development requires `nats-server` installed.
- Slightly higher barrier to entry for new developers (mitigated by `vlinder daemon` managing NATS lifecycle).
- Integration tests that currently run in-memory will need NATS or must stay on InMemoryQueue as an explicit test fixture.
