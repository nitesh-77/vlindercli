# ADR 065: DAG Workers as Projections

## Status

Accepted

## Context

ADR 064 proposed git as the unified store — one backend to replace both ConversationStore and DagStore. That framing was wrong. Not because git is wrong, but because "the store" is wrong.

The DagCaptureWorker (ADR 062) is a NATS listener. It subscribes to `vlinder.>`, pairs invoke/complete messages, and writes DAG nodes. It doesn't care where it writes. Today it writes to SQLite via `Box<dyn DagStore>`. It could just as easily write to git. Or Elasticsearch. Or a webhook. Or all of them.

The worker pattern is the real abstraction — not the storage backend.

### Different stores serve different needs

**SQLite** is good at analytical queries: aggregate across sessions, filter by agent, count invocations, power dashboards and web UIs. It's a reporting store.

**Git** is good at exploration: diff two runs, bisect where behavior diverged, fork and replay, share a session via `git push`, browse in any git client. It's a developer's debugging tool.

Neither replaces the other. They serve different audiences with different access patterns. Choosing one is a false dilemma — they're both projections of the same event stream.

### NATS is the source of truth

Every agent interaction already flows through NATS. The invoke/complete pairs are the canonical events. Any system that can subscribe to a NATS subject can build its own materialized view of agent activity. No platform changes required. No plugin API. Just NATS.

### Agent-authored commit trees

In the git projection, the commit tree doesn't have to follow a fixed schema. The platform writes `payload_in` and `payload_out` blobs as the minimum. But the agent author decides what else goes in: error logs, structured output, screenshots, diagnostic bundles — whatever artifacts the agent produces. The commit message stays human-readable for `git log`. The tree is the agent's artifact space.

## Decision

**DAG workers are NATS projections.** Each worker subscribes to the same event stream and materializes a different view. The platform ships two:

1. **SqliteDagWorker** — analytical projection for queries, dashboards, and reporting
2. **GitDagWorker** — exploration projection for time travel, diffing, forking, and developer tools

Both implement the same worker trait: receive invoke/complete pairs from NATS, write to their backend. The trait is the NATS listener pattern, not the storage interface.

### Worker trait

```rust
pub trait DagWorker: Send + Sync {
    fn on_invoke(&mut self, ctx: &InvokeContext, payload: &[u8]);
    fn on_complete(&mut self, ctx: &CompleteContext, payload: &[u8]);
}
```

The existing `DagCaptureWorker` becomes `SqliteDagWorker`. A new `GitDagWorker` writes commits. Both subscribe to `vlinder.>`. Both run as queue workers. Neither knows about the other.

### Fan-out

```
NATS (vlinder.>)
    ├──→ SqliteDagWorker  → queries, web UI, reports
    ├──→ GitDagWorker     → time travel, exploration, sharing
    └──→ ???              → anything that subscribes to NATS
```

Adding a new projection means writing a new worker and subscribing it. No platform changes. No configuration. Just another NATS consumer.

### What changes from ADR 064

ADR 064 positioned git as THE unified store. This ADR corrects that: git is ONE projection. SQLite is another. NATS is the source of truth. The DagStore trait stays as the SQLite storage interface. The git projection uses git's own interface — commits, trees, blobs — not the DagStore trait.

## Consequences

- No single store to choose — each projection serves its audience
- SQLite stays for analytical queries; git stays for developer exploration
- NATS is the integration point, not a storage API
- Third-party projections require zero platform changes — subscribe to NATS
- The DagCaptureWorker generalizes into a worker trait with multiple implementations
- Agent authors control what goes in their git commit trees — the platform doesn't prescribe artifact format
- The platform is a fan-out event processor, not a database application
