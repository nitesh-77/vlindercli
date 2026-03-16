# ADR 116: State on Every DAG Node

**Status:** Draft

## Context

State is currently a field on specific message variants (Complete, Invoke, Request) inside `ObservableMessage`. A DagNode accesses state transitively via `node.message.state()`. This has three problems:

1. **Not every node carries state.** Only Complete messages reliably have a state hash. Request/Response nodes may or may not. Invoke nodes carry the *previous* Complete's state. There is no guarantee that an arbitrary node tells you the full storage snapshot at that point.

2. **New sessions bypass versioned reads.** When `initial_state` is `None`, the KV worker falls through to an unversioned read of the raw SQLite table. This leaks accumulated state from all prior sessions instead of presenting the correct snapshot.

3. **State is message-specific, not positional.** In git, every commit points to a tree — the full snapshot at that point, regardless of what changed. In our DAG, state is a property of what happened (the message), not where it happened (the node's position in the DAG).

### Git analogy

| Git             | Vlinder DAG        |
|-----------------|--------------------|
| Commit hash     | DagNodeId          |
| Tree hash       | StateTreeId        |
| Parent commit   | parent_id          |
| Commit message  | ObservableMessage  |
| Tree object     | StateTree          |
| Blob hash       | per-store StateHash|

A commit *contains* a tree hash but they are different values. The commit hash includes the tree in its computation. The tree is the snapshot; the commit is the event. Same relationship should hold for DagNodeId and state.

## Decision

### State tree as a content-addressed object

Each store produces its own state hash independently. A KV put returns a KV state hash. A vector store returns a vector state hash. These per-store hashes are collected into a `StateTree` — a content-addressed object analogous to a git tree:

```rust
/// Content-addressed state tree — maps store names to their state hashes.
/// Analogous to a git tree object mapping file names to blob hashes.
pub struct StateTree {
    pub stores: BTreeMap<String, StateHash>,
}

impl StateTree {
    /// Deterministic hash of sorted entries. BTreeMap guarantees key order,
    /// so the same entries always produce the same StateTreeId.
    pub fn id(&self) -> StateTreeId { ... }
}
```

`BTreeMap` (not `HashMap`) because sorted keys give deterministic serialization, which gives stable content addressing.

### State is a DagNode field

`state` moves from `ObservableMessage` variants to `DagNode` itself. The node carries a single `StateTreeId` — a pointer to the state tree at that point in the DAG:

```rust
pub struct DagNode {
    pub id: DagNodeId,
    pub parent_id: DagNodeId,
    pub created_at: DateTime<Utc>,
    pub state: StateTreeId,     // <- content-addressed pointer to StateTree
    pub message: ObservableMessage,
}
```

Every node carries a state tree. No exceptions. Content-addressed all the way down: DagNodeId → StateTreeId → per-store StateHash → snapshots → values.

### Inheritance

If a node does not modify any store, it reuses the parent's `StateTreeId` directly — same hash, zero cost. If one store changed, a new `StateTree` is created with the updated hash for that store and inherited hashes for the rest, producing a new `StateTreeId`.

Root nodes (no parent) point to the empty state tree — a tree where all stores map to the empty snapshot hash.

### New session state

A new session (no `--session` flag) inherits the state tree from the agent's most recent session's main branch tip. This is the accumulated state across all prior sessions on main — analogous to starting new work from HEAD.

`None` is no longer a valid initial state. A new session either inherits prior state or starts with the empty state tree (first session ever).

### Branching semantics

- **New session**: continues main's HEAD state tree
- **Resume (`--session`)**: continues that branch's HEAD state tree
- **Fork**: restores state tree from fork point node
- **Promote**: fork's HEAD state tree becomes main's new HEAD

These follow git semantics. The platform presents the state; the agent doesn't need to know about state trees or hashes. `kv_get` and `kv_put` go through the sidecar, which scopes reads to the correct snapshot using the node's state.

### DagNodeId computation

The state tree ID is included in the DagNodeId computation, just as a git commit hash includes the tree hash:

```
DagNodeId = SHA-256(parent_id || message_blob || state_tree_id || created_at)
```

Two nodes with the same message but different state produce different IDs.

## Consequences

- `ObservableMessage` variants lose their `state` field
- `DagNode` gains a `state: StateTreeId` field, always present
- `StateTree` and `StateTreeId` are new domain types
- The harness no longer passes `None` for initial state
- The KV worker's unversioned fallback path becomes dead code
- `node.message.state()` calls throughout the codebase become `node.state`
- The state trailer concept in git-dag commit messages simplifies to reading the node's state tree ID directly
