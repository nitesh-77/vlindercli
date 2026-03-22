# ADR 122: SQL Representation of the Vlinder Protocol

**Status:** Draft

## Context

The vlinder protocol captures every agent interaction as a content-addressed node in a Merkle DAG. The protocol defines typed messages — Invoke, Request, Response, Complete, Delegate, Repair, Fork, Promote — each with distinct fields. They share common structure: session, branch, submission, sender, receiver, and a hash that chains them.

We need a SQL schema that represents this protocol. The schema should reflect the protocol's type structure so that:

- Each message type's fields are explicit and enforced
- New message types extend the schema without modifying existing tables
- Queries can filter and join on type-specific fields
- The repository interface expresses consumer questions, not internal extraction

## Decision

### Existing `dag_nodes` table stays as-is

The existing 18-column `dag_nodes` table handles DAG structure, display, and the Merkle chain. It works. Cleaning up its polymorphic columns is a future concern.

### Add type tables for message-specific fields

Each message type gets a table with the fields consumers actually read. These tables store just enough to hydrate the entity without deserializing `message_blob`.

**`dag_invoke_nodes`**

| Column | Type | Description |
|--------|------|-------------|
| hash | TEXT PK FK → dag_nodes | |
| state | TEXT | KV state hash |
| diagnostics | BLOB | Serialized InvokeDiagnostics |

**`dag_request_nodes`**

| Column | Type | Description |
|--------|------|-------------|
| hash | TEXT PK FK → dag_nodes | |
| state | TEXT | KV state hash |
| operation | TEXT | get, put, run, search, etc. |
| checkpoint | TEXT | Durable mode checkpoint |
| diagnostics | BLOB | Serialized RequestDiagnostics |

**`dag_response_nodes`**

| Column | Type | Description |
|--------|------|-------------|
| hash | TEXT PK FK → dag_nodes | |
| state | TEXT | KV state hash |
| operation | TEXT | Mirrors the request |
| status_code | INTEGER | HTTP status |
| correlation_id | TEXT | Request message ID |
| diagnostics | BLOB | Serialized ServiceDiagnostics |

**`dag_complete_nodes`**

| Column | Type | Description |
|--------|------|-------------|
| hash | TEXT PK FK → dag_nodes | |
| state | TEXT | KV state hash |
| stderr | BLOB | Container stderr |
| diagnostics | BLOB | Serialized RuntimeDiagnostics |

**`dag_delegate_nodes`**

| Column | Type | Description |
|--------|------|-------------|
| hash | TEXT PK FK → dag_nodes | |
| state | TEXT | KV state hash |
| nonce | TEXT | Delegation nonce |
| diagnostics | BLOB | Serialized DelegateDiagnostics |

**`dag_repair_nodes`**

| Column | Type | Description |
|--------|------|-------------|
| hash | TEXT PK FK → dag_nodes | |
| operation | TEXT | Service operation being replayed |
| checkpoint | TEXT | Checkpoint being repaired |
| sequence | INTEGER | Sequence number |

Fork and Promote have no type-specific fields. No type table needed.

### Repository interface

Rename `DagStore` to `DagRepository`. Grow the interface from what consumers ask for:

```rust
pub trait DagRepository: Send + Sync {
    fn insert_node(&self, node: &DagNode) -> Result<(), String>;
    fn get_node(&self, id: &DagNodeId) -> Result<Option<DagNode>, String>;
    fn get_session_nodes(&self, session_id: &SessionId) -> Result<Vec<DagNode>, String>;
    fn get_children(&self, parent_id: &DagNodeId) -> Result<Vec<DagNode>, String>;
    fn latest_node_on_branch(&self, branch: BranchId, msg_type: Option<MessageType>) -> Result<Option<DagNode>, String>;
    fn latest_state_on_branch(&self, branch: BranchId) -> Result<Option<String>, String>;
    fn list_sessions(&self) -> Result<Vec<SessionSummary>, String>;
    // branch and session management methods...
}
```

Add type-specific query methods when consumers need them, not before.

## Implementation Plan (stacked diffs)

Each step is a separate branch, independently e2e-green. Steps go
on main as preparatory refactors. After merging, the invoke-v2
migration branches (ADR 121 steps 7-10) rebase cleanly.

### 1. `fix-cli-polymorphic-accessors` (done)

CLI commands match on `ObservableMessage` variants directly instead
of calling `node.message.operation()`, `node.message.checkpoint()`.

### 2. `add-type-tables`

Create type tables in `SqliteDagStore::open()`. Existing `dag_nodes`
table untouched. Clean slate — no data migration.

### 3. `write-type-tables`

`insert_node` writes to the type table alongside the existing
`dag_nodes` row. Matches on `ObservableMessage` variant to determine
which type table and which fields. The match is in the storage layer
— one place.

### 4. `read-type-tables`

`latest_state_on_branch` queries the type table for state hash
instead of going through `node.message.state()`. Harness uses the
new method. Other read paths stay on `dag_nodes` for now — they
don't need type-specific fields.

### 5. `fix-storage-polymorphic-accessors`

`insert_node` stops using `node.message.from_to()`,
`node.message.diagnostics_json()`, etc. Matches on variant directly
for the `dag_nodes` denormalized columns too.

### 6. `fix-list-sessions`

`list_sessions` reads agent name from `receiver` column on
`dag_nodes` row (already there), not via `node.message.from_to().1`.

### 7. `remove-polymorphic-accessors`

Delete `operation()`, `checkpoint()`, `from_to()`, `state()`,
`diagnostics_json()`, `stderr()` from `ObservableMessage`.
Compiler confirms no consumers remain.

### Open question

`insert_node` currently takes `&DagNode` which contains
`ObservableMessage`. The match on variant happens inside the storage
impl. When `DagNode` eventually stops holding `ObservableMessage`
(future step), the typed message needs to reach the storage layer
another way. Options:
- Deserialize from `message_blob` (wasteful but simple)
- Change trait to `insert_node(&DagNode, &ObservableMessage)`
- Type-aware insert methods on the repository

Defer this decision. For now `DagNode` still holds the message.

### Deferred

- **Merkle DAG representation in SQL.** The `dag_nodes` base table has hash/parent_hash columns that form the chain. The schema works but doesn't enforce chain integrity or optimize for DAG traversal. Revisit when it's a real performance or correctness problem.
- **`DagNode` struct cleanup.** Removing `ObservableMessage` from `DagNode` and replacing with flat fields. Blocked on the open question above. Tackle after the type tables are proven.
- **`DagStore` → `DagRepository` rename.** Cosmetic. Do it when touching the trait for other reasons.

## Consequences

- Schema mirrors the protocol's type structure
- Type-level invariants enforced by table structure
- New message types = new type table, no changes to existing tables
- `DagNode` struct carries base table fields, not a polymorphic enum
- Repository stops cracking open message variants
- Unblocks protocol evolution (ADR 121)
