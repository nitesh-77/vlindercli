# ADR 063: Route as Domain Model

## Status

Proposed

## Context

When a user talks to a fleet, many conversations happen. The user asks the entry agent a question. The entry agent delegates to a log-analyst. The log-analyst responds. The entry agent delegates to a code-analyst. The code-analyst responds. The entry agent synthesizes everything and replies to the user.

Five conversations happened. The user only sees one: their question and the final answer. Everything in between is invisible.

The Session model (ADR 054) captures the user↔entry-agent exchange. It records `HistoryEntry::User` and `HistoryEntry::Agent` turns for a single agent. It doesn't know about delegations. From the user's perspective, a fleet is a black box.

The DAG (ADR 061) already captures every agent boundary — every invoke/complete pair, including internal delegations. The data exists. Nothing surfaces it to the user yet.

### The gap

A conversation is any request↔response exchange between two parties. It doesn't have to be user↔agent — it can be agent↔agent. In a fleet, the full picture is a chain of conversations:

```
user → support-agent          (conversation 1)
  support-agent → log-analyst   (conversation 2)
  support-agent → code-analyst  (conversation 3)
support-agent → user          (conversation 4)
```

Each arrow is a conversation. The ordered sequence is the **route** — every stop a request hits between the user's question and the user's answer.

### Why a domain model

The route is a concept that multiple consumers need:

- `vlinder timeline route` (CLI display)
- The support fleet's log-analyst (programmatic query)
- Future: web harness, API, export

If Route is rendering logic in the command layer, every consumer reimplements the same interpretation. If it's a domain type, consumers format; they don't interpret.

## Decision

**Route is a domain type that represents the full chain of conversations a request triggers — every agent stop between user input and user output.** It is constructed from DAG nodes and makes fleet internals visible.

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

Each Stop is one conversation: someone sent `payload_in`, the agent returned `payload_out`. A Route is the ordered sequence of all conversations in a session.

A single-agent session is a Route with one stop. A fleet produces a Route with many stops. The model is the same.

### Construction

```rust
impl Route {
    pub fn from_dag_nodes(session_id: String, nodes: Vec<DagNode>) -> Self;
}
```

The factory takes the session ID and the ordered nodes from the DAG store. No DagStore dependency in the domain type — the caller queries the store and passes the result. Construction is a projection: strip Merkle chain details (parent_hash), keep what describes the stop.

### What Route provides

| Method | Returns | Purpose |
|--------|---------|---------|
| `stops()` | `&[Stop]` | Ordered stops |
| `stop_count()` | `usize` | Number of conversations in the chain |
| `agents()` | `Vec<&str>` | Unique agents in invocation order |
| `duration_secs()` | `Option<f64>` | Elapsed time from first to last stop |

### CLI surface

```
vlinder timeline route <session_id>
```

Renders the route as a vertical diagram showing each stop with agent name, elapsed time, hash prefix, and a truncated payload preview. The user sees every conversation that happened, not just the outer one.

### File location

`src/domain/route.rs` — alongside other domain types.

## Consequences

- Fleet internals become visible to the user — every agent-to-agent conversation is surfaced
- Route is a domain type, not a presentation concern — any harness can render it
- A single-agent session and a fleet session use the same model — Route generalizes both
- DagStore remains a storage concern; Route doesn't depend on it
- The `timeline route` command becomes thin: query DagStore, construct Route, format output
- Future analytical operations (diff two routes, find divergence point) compose naturally on top of Route
