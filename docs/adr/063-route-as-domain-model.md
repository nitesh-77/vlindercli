# ADR 063: Route as Domain Model

## Status

Proposed

## Context

ADR 061 introduced the runtime-captured DAG: a worker observes invoke/complete pairs from the NATS stream and writes content-addressed nodes into SQLite. The data is being collected, but nothing consumes it yet.

The most common analytical question for fleets is: **what route did a request take?** A user submits input, the entry agent delegates to specialists, specialists respond, and the entry agent synthesizes a final answer. Each agent invocation is a stop. The ordered sequence of stops is the route.

The DAG already captures this — `get_session_nodes(session_id)` returns every invoke/complete pair in a session, ordered by timestamp. But there's no domain concept that gives these nodes meaning as a route.

### Why a domain model, not just a CLI formatter

The route is a concept that multiple consumers need:

- `vlinder timeline route` (CLI display)
- The support fleet's log-analyst (programmatic query)
- Future: web harness, API, export

If Route is just rendering logic in the command layer, every consumer reimplements the same interpretation. If it's a domain type, consumers format; they don't interpret.

## Decision

**Route is a domain type that represents the ordered sequence of agent stops a request takes through a fleet.** It is constructed from DAG nodes and provides the vocabulary for inspection.

### Types

```rust
pub struct Route {
    pub session_id: String,
    pub stops: Vec<Stop>,
}

pub struct Stop {
    pub hash: String,
    pub agent: String,
    pub payload_in: Vec<u8>,
    pub payload_out: Vec<u8>,
    pub created_at: String,
}
```

A Route is derived from a `Vec<DagNode>` for a session. Construction is a projection: strip the Merkle chain details (parent_hash), keep what describes the stop.

### Construction

```rust
impl Route {
    pub fn from_dag_nodes(session_id: String, nodes: Vec<DagNode>) -> Self;
}
```

The factory takes the session ID and the ordered nodes from the DAG store. No DagStore dependency in the domain type — the caller queries the store and passes the result.

### What Route provides

| Method | Returns | Purpose |
|--------|---------|---------|
| `stops()` | `&[Stop]` | Ordered stops |
| `stop_count()` | `usize` | Number of agent invocations |
| `agents()` | `Vec<&str>` | Unique agents in invocation order |
| `duration_secs()` | `Option<f64>` | Elapsed time from first to last stop |

### CLI surface

```
vlinder timeline route <session_id>
```

Renders the route as a vertical diagram showing each stop with agent name, elapsed time, hash prefix, and a truncated payload preview.

### File location

`src/domain/route.rs` — alongside other domain types.

## Consequences

- Route is a domain type, not a presentation concern — any harness can render it
- DagStore remains a storage concern; Route doesn't depend on it
- The `timeline route` command becomes thin: query DagStore, construct Route, format output
- Future analytical operations (diff two routes, find divergence point) compose naturally on top of Route

