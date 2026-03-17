# ADR 116: State on Every DAG Node

**Status:** Accepted

## Context

State is currently a field on specific message variants (Complete, Invoke, Request) inside `ObservableMessage`. A DagNode accesses state transitively via `node.message.state()`. This has three problems:

1. **Not every node carries state.** Only Complete messages reliably have a state hash. Request/Response nodes may or may not. Invoke nodes carry the *previous* Complete's state. There is no guarantee that an arbitrary node tells you the full storage snapshot at that point.

2. **New sessions bypass versioned reads.** When `initial_state` is `None`, the KV worker falls through to an unversioned read of the raw SQLite table. This leaks accumulated state from all prior sessions instead of presenting the correct snapshot.

3. **State is message-specific, not positional.** In git, every commit points to a tree — the full snapshot at that point, regardless of what changed. In our DAG, state is a property of what happened (the message), not where it happened (the node's position in the DAG).

### Git analogy

| Git             | Vlinder DAG        |
|-----------------|--------------------|
| Commit hash     | DagNodeId          |
| Tree object     | Snapshot           |
| Parent commit   | parent_id          |
| Commit message  | ObservableMessage  |
| Blob hash       | per-store StateHash|

## Decision

### Snapshot as the state container

Each store produces its own state hash independently. A KV put returns a KV state hash. A vector store returns a vector state hash. These per-store hashes are collected into a `Snapshot` — a sorted map analogous to a git tree:

```rust
/// Maps store instances to their state hashes.
/// BTreeMap guarantees key order for deterministic serialization.
pub struct Snapshot(pub BTreeMap<Instance, StateHash>);
```

`BTreeMap` (not `HashMap`) because sorted keys give deterministic serialization.

### State is a DagNode field

`state` lives on `DagNode` itself. Every node carries a `Snapshot`:

```rust
pub struct DagNode {
    pub id: DagNodeId,
    pub parent_id: DagNodeId,
    pub created_at: DateTime<Utc>,
    pub state: Snapshot,
    pub message: ObservableMessage,
}
```

Every node carries a snapshot. No exceptions.

**Note:** Message variants (Invoke, Request, Complete) still carry `state: Option<String>` for transport. The `RecordingQueue` uses the message's state to build the node's `Snapshot` via `Snapshot::with_state()`, merging with the parent's snapshot. The message field is the input; the node's `Snapshot` is the computed result.

### Inheritance

If a node does not modify any store, it reuses the parent's `Snapshot` directly — same value, zero cost. If one store changed, a new `Snapshot` is created with the updated hash for that store and inherited hashes for the rest.

Root nodes (no parent) start with an empty snapshot.

### Branching semantics

- **New session**: starts with empty snapshot (or inherits from prior session's main tip)
- **Resume (`--session`)**: continues that branch's HEAD snapshot
- **Fork**: restores snapshot from fork point node
- **Promote**: fork's HEAD snapshot becomes main's new HEAD

The platform presents the state; the agent doesn't need to know about snapshots or hashes. `kv_get` and `kv_put` go through the sidecar, which scopes reads to the correct snapshot using the node's state.

## Consequences

- `DagNode` has a `state: Snapshot` field, always present
- `Snapshot`, `Instance`, `StateHash` are domain types in `message/identity.rs`
- State inheritance is handled in `build_dag_node()` — merges message state with parent snapshot
- Message variants retain `state: Option<String>` as transport (not yet removed)
- The KV worker scopes reads using the snapshot from the DAG
