# Bring Your Own Storage

How to integrate external storage backends (Postgres, Qdrant, etc.) with
vlinder's versioned state model.

## The Problem

Vlinder's state model (ADR 055) gives you crash safety, forking, and
time travel. Today it works with platform-provided SQLite storage. But
agents may need Postgres for relational data, Qdrant for production
vector search, Redis for caching, or any other backend.

The question: how does an external backend participate in the three rules?

## The Three Rules (recap)

1. **Immutable objects:** writes create new data, never overwrite
2. **Content addressing:** objects identified by hash of content
3. **Platform-managed pointers:** "current state" is a ref the platform
   controls

Today the platform implements all three rules internally: the
`StateStore` (SQLite) stores values/snapshots/commits, and the
`ObjectServiceWorker` threads state hashes through the invocation.

With external backends, the responsibility splits.

## The Split: Platform Manages Pointers, Agent Manages Data

```
┌────────────────────────────────────────────────────────┐
│                    Platform                            │
│                                                        │
│  - Provides the current state hash at invocation start │
│  - Records the final state hash at invocation end      │
│  - Manages session branches (fork/promote)             │
│  - Does NOT touch the agent's data                     │
└──────────────────────┬─────────────────────────────────┘
                       │ state hash
┌──────────────────────▼─────────────────────────────────┐
│                     Agent                              │
│                                                        │
│  - Receives current state hash                         │
│  - Reads/writes in its own backend                     │
│  - Follows the three rules in its backend              │
│  - Reports new state hash back to platform             │
└────────────────────────────────────────────────────────┘
```

The platform doesn't need to know about Postgres or Qdrant. It just
needs a hash string that represents the agent's state. The agent is
responsible for making that hash meaningful.

## What the Agent Must Do

### 1. Accept a state hash on invocation

The agent receives a `state` field in each request. This is the hash
from the last successful invocation (or empty string for first run).

### 2. Use the state hash to resolve current data

The agent must be able to reconstruct its view of the world from any
historical state hash. This is the "content addressing" rule: the hash
IS the state, not a pointer to mutable data.

### 3. Make writes append-only

The agent must not overwrite or delete data in its backend. New writes
create new records. Old records remain accessible by their hashes.

### 4. Report the new state hash back

After all writes, the agent returns a state hash that captures the final
state. The platform records this hash as the `State:` trailer.

## Example: Postgres as KV Storage

### Schema

```sql
-- Values: content-addressed blobs
CREATE TABLE agent_values (
    hash    TEXT PRIMARY KEY,    -- SHA256(content)
    content BYTEA NOT NULL
);

-- Snapshots: path→hash mappings at a point in time
CREATE TABLE agent_snapshots (
    hash    TEXT PRIMARY KEY,    -- SHA256(sorted JSON of entries)
    entries JSONB NOT NULL       -- {"path": "value_hash", ...}
);

-- State commits: snapshot + parent (the chain)
CREATE TABLE agent_state_commits (
    hash          TEXT PRIMARY KEY,  -- SHA256(snapshot_hash + ":" + parent_hash)
    snapshot_hash TEXT NOT NULL REFERENCES agent_snapshots(hash),
    parent_hash   TEXT NOT NULL,     -- "" for root
    created_at    TIMESTAMPTZ DEFAULT NOW()
);
```

This is the same three-table structure as the platform's SQLite
`StateStore`, just in Postgres. The schema is identical because the
model is identical.

### Write path

```
kv_put("/todos.json", '["buy milk"]', parent_state="sc1")

1. hash = SHA256('["buy milk"]')
2. INSERT INTO agent_values (hash, content) VALUES ($hash, $content)
   ON CONFLICT DO NOTHING
3. Load parent snapshot from agent_snapshots WHERE hash = (
     SELECT snapshot_hash FROM agent_state_commits WHERE hash = 'sc1'
   )
4. Clone entries, set "/todos.json" → $hash
5. snapshot_hash = SHA256(sorted JSON of new entries)
6. INSERT INTO agent_snapshots ON CONFLICT DO NOTHING
7. commit_hash = SHA256(snapshot_hash + ":" + "sc1")
8. INSERT INTO agent_state_commits ON CONFLICT DO NOTHING
9. Return commit_hash to platform
```

### Read path

```
kv_get("/todos.json", state="sc2")

1. SELECT snapshot_hash FROM agent_state_commits WHERE hash = 'sc2'
2. SELECT entries FROM agent_snapshots WHERE hash = $snapshot_hash
3. Look up "/todos.json" in entries → value_hash
4. SELECT content FROM agent_values WHERE hash = $value_hash
```

Three hops, same as SQLite. Postgres just runs them faster at scale and
gives you connection pooling, replication, and MVCC for free.

### What forking looks like

When the user runs `vlinder session fork`, the platform creates a
new branch in the session. The next invocation receives the state
hash from the fork point. The agent's Postgres tables don't change; the agent just starts
reading from an older state commit. Since everything is append-only, the
old data is still there.

New writes on the fork create new state commits with the forked state as
parent. Both timelines coexist in the same Postgres tables, just like
they coexist in the same SQLite `state.db` today.

## Example: Qdrant as Vector Storage

Vector storage is trickier because similarity search is inherently
mutable: you search the current index, not a historical snapshot.

### The approach: snapshot-scoped collections

```
Collection naming: {agent}_{snapshot_hash_prefix}

Fork from sc2:
  - Original collection: todoapp_sc3_abcd  (has all embeddings up to sc3)
  - Fork collection:     todoapp_sc2_efgh  (has embeddings up to sc2)
```

Each state commit references a Qdrant collection name. Forking means
creating a new collection (or using Qdrant's snapshot feature) from the
state at the fork point.

### Write path

```
embed("chunk-0", vector, metadata, parent_state="sc2")

1. Determine current collection from sc2's snapshot
2. Upsert point into the collection
3. Create new snapshot entry: {"chunk-0" → point_id, ...}
4. snapshot_hash = SHA256(sorted entries)
5. commit_hash = SHA256(snapshot_hash + ":" + "sc2")
6. Return commit_hash
```

### Search path

```
search(query_vector, limit=5, state="sc3")

1. Determine collection from sc3's snapshot
2. Search within that collection
3. Return results
```

### The hard part: collection proliferation

Unlike KV where reads are point lookups by hash, vector search requires
an indexed collection. Each fork potentially needs its own collection.
Strategies:

- **Copy-on-fork**: snapshot the Qdrant collection when forking. Simple
  but expensive for large collections.
- **Filtered search**: one collection, tag each point with the state
  commit that added it. Search with a filter for all state commits
  reachable from the current state. Elegant but requires walking the
  commit chain.
- **Lazy materialization**: only create a fork collection when the fork
  actually writes new embeddings. Until then, search the parent
  collection.

This is an open design problem. The platform's SQLite vector storage
doesn't support versioned search yet (it's in the "Deferred" section of
ADR 055).

## The Contract

Regardless of backend, the contract between agent and platform is:

| Responsibility | Owner |
|----------------|-------|
| Provide current state hash at invocation start | Platform |
| Record final state hash at invocation end | Platform |
| Manage session branches (fork/promote) | Platform |
| Store and retrieve data | Agent |
| Ensure append-only writes | Agent |
| Compute deterministic hashes | Agent |
| Resolve any historical state hash to data | Agent |

The platform publishes the hash algorithm (SHA-256). The agent implements
the three rules in whatever backend it chooses. The state hash is the
interface between them.

## What Doesn't Exist Yet

This doc describes the target design. Today, all storage goes through the
platform's `ObjectServiceWorker` and `StateStore`. For agents to bring
their own storage, we need:

1. **SDK calls for state reporting:** `state_advance(hash)` and
   `state_resolve()` so agents can report their own state hashes through
   the platform's message protocol.
2. **Manifest declaration:** a way to declare `object_storage = "postgres://..."`
   or `vector_storage = "qdrant://..."` in `agent.toml` so the platform
   knows the agent manages its own storage.
3. **State hash validation:** the platform should verify that state
   hashes are deterministic (same input = same hash) to prevent bugs
   where an agent returns non-reproducible hashes.

These are tracked as future work in ADR 055.

## Why This Matters

The state model isn't tied to SQLite. It's three rules and a hash. Any
backend that follows the rules gets crash safety, forking, and time
travel for free, because those capabilities come from the model, not
the storage engine. Postgres, Qdrant, Redis, S3, a flat file... the
mechanism changes, the guarantees don't.
