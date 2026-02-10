# ADR 064: Unified DAG Storage

## Status

Proposed

## Context

Vlinder has two stores that capture chains of content-addressed nodes:

**ConversationStore** (ADR 054) — a git repo at `~/.vlinder/conversations/`. Each commit is a user message or agent response. Parent links form a chain. Fork is `git checkout -b`. Only sees user↔entry-agent turns.

**DagStore** (ADR 061) — SQLite with a `dag_nodes` table. Each node is an invoke/complete pair. `parent_hash` links form a chain. Sees all agent boundaries including internal delegations.

They are structurally identical:

| | ConversationStore | DagStore |
|--|--|--|
| Unit | Git commit | DagNode |
| Identity | SHA-1 | SHA-256 |
| Chain | Parent commit | parent_hash |
| Scope | Per session | Per session |
| Append-only | Yes | Yes |
| Fork | git checkout -b | Not yet implemented |

Both are content-addressed, append-only, parent-linked chains scoped by session. One uses git as a backend, the other uses SQLite. Both are DAGs.

### The problem

Two stores for the same shape of data creates friction:

- The conversation store only sees the outer conversation. Fleet internals are invisible (the gap ADR 063 identified).
- The DAG store sees everything but can't fork — that's a git operation.
- Route (ADR 063) needs the DAG's completeness. Session needs the conversation store's operational role. They should be the same data.
- Two write paths, two schemas, two failure modes for what is conceptually one chain.

### What git provides today

- Content-addressed commits (SHA-1)
- Parent links (commit chain)
- Branching (`git checkout -b` for fork)
- Log (`git log` for timeline)
- Native tooling (`git diff`, `git branch`, etc.)

### What SQLite provides today

- Content-addressed nodes (SHA-256)
- Parent links (parent_hash column)
- Indexed queries (by session, by agent, by parent)
- Recursive CTEs for ancestry walks
- WAL mode for concurrent access

Everything git provides can be expressed as DAG operations on SQLite. Fork = insert a new node with an existing node as parent. Log = query by session ordered by timestamp. Branch tracking = a refs table mapping branch names to node hashes.

## Decision

**The DagStore trait becomes the unified interface for all chain-structured, content-addressed storage.** The git-backed ConversationStore is replaced by a SQLite-backed implementation of the same trait.

### What changes

- `DagStore` gains fork and ref-tracking operations
- `ConversationStore` is replaced — session data is stored as DagNodes
- The git repo at `~/.vlinder/conversations/` is no longer needed
- One database, one schema, one write path

### Extended DagStore trait

```rust
pub trait DagStore: Send + Sync {
    // Existing
    fn insert_node(&self, node: &DagNode) -> Result<(), String>;
    fn get_node(&self, hash: &str) -> Result<Option<DagNode>, String>;
    fn get_session_nodes(&self, session_id: &str) -> Result<Vec<DagNode>, String>;
    fn get_children(&self, parent_hash: &str) -> Result<Vec<DagNode>, String>;

    // New: ref tracking (replaces git branches)
    fn set_ref(&self, name: &str, hash: &str) -> Result<(), String>;
    fn get_ref(&self, name: &str) -> Result<Option<String>, String>;
    fn list_refs(&self) -> Result<Vec<(String, String)>, String>;

    // New: ancestry (replaces git log)
    fn ancestors(&self, hash: &str) -> Result<Vec<DagNode>, String>;
}
```

Refs replace git branches. `main` points to the latest node. Fork creates a new ref pointing to an existing node. Switching branches = changing which ref the harness reads from.

### Schema additions

```sql
-- Refs: named pointers to nodes (like git branches)
CREATE TABLE dag_refs (
    name TEXT PRIMARY KEY,
    hash TEXT NOT NULL REFERENCES dag_nodes(hash)
);
```

### What doesn't change

- DagNode structure stays the same
- Content addressing stays the same (SHA-256)
- Merkle chain property stays the same
- The DagCaptureWorker still writes nodes from NATS traffic
- Route (ADR 063) constructs from DagNodes — unaffected

## Consequences

- One store instead of two — conversation and DAG data live in the same database
- Fleet internals and user conversations are stored uniformly
- Fork is a DAG operation (new ref pointing to existing node), not a git operation
- Git dependency removed from the operational path
- Route, diff, bisect, fork all operate on the same data through the same trait
- The `git log`/`git show` debugging escape hatch is lost — replaced by `vlinder timeline` commands that query SQLite
- Migration path needed for existing conversation repos (or clean break for pre-1.0)
