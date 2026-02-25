# ADR 106: Replace sqlite-vec with pgvector

## Status

Draft

## Context

sqlite-vec validated the provider hostname pattern and proved vector storage works end-to-end without external dependencies. But it has a fundamental limitation: the `vec0` virtual table cannot combine vector search (`embedding MATCH ?`) with arbitrary WHERE clauses.

This blocks time travel for vector storage. When an agent checks out a previous state, the platform needs to search only vectors that existed at that state. The mechanism (ADR 055): each write advances the state chain, the snapshot captures which vectors existed, and search filters to that set.

For KV this works — `versioned_get` and `versioned_list` resolve through the snapshot. But KV operations are key lookups, not scans. Vector search computes distances across all stored vectors and returns the nearest. Filtering must happen *during* the search, not after, to get correct nearest-neighbor rankings.

sqlite-vec can't do this. It's a virtual table with a restricted query planner. You can't add `WHERE state_created IN (...)` alongside `embedding MATCH ?`.

This is not a fundamental limitation of vector search. pgvector, Qdrant, Pinecone, Weaviate, and Milvus all support filtered vector search natively. pgvector is a regular Postgres extension — full SQL works:

```sql
SELECT key, metadata, embedding <-> $1 AS distance
FROM vectors
WHERE state_created = ANY($2)
ORDER BY distance
LIMIT $3
```

### Why pgvector

- Full SQL — WHERE clauses, JOINs, subqueries all work alongside vector distance operators
- HNSW and IVFFlat indexes support pre-filtering
- Postgres is a dependency agents already expect in production
- Single process for dev (local install), managed Postgres for production
- Ancestry sets can be pre-materialized as arrays or tables, indexed efficiently

### What sqlite-vec can't do that matters

| Capability | sqlite-vec | pgvector |
|---|---|---|
| Filtered search | No | Yes |
| Time-travel search | Post-filter only | Native filter |
| Accurate top-N with filter | No (over-fetch and hope) | Yes |
| Index-level pre-filtering | No | Yes (HNSW) |

## Decision

Replace `vlinder-sqlite-vec` with `vlinder-pgvector`.

### Hostname

`pgvector.vlinder.local` — hostnames name the backend (ADR 100).

### Same API surface, different backend

The agent-facing operations stay the same: `store`, `search`, `delete`. Request and response types don't change. Only the hostname and manifest change:

```toml
vector_storage = "postgres://data/vectors"
```

### Time travel with filtered search

Vector writes advance the state chain (ADR 055). Each `store` and `delete` creates a state commit. The snapshot at any state captures which vectors exist.

On time-travel search, the worker computes the ancestry set and passes it as a native WHERE clause. pgvector's HNSW index handles this efficiently — the filter is applied during index traversal, not as a post-filter.

### Soft deletes

Vectors are never physically removed. Delete marks `soft_deleted = true` and records the state hash. Time travel to a state before the delete sees the vector; after the delete, it's filtered out.

### Development story

For local dev, use a local Postgres with pgvector extension. The `justfile` gets a recipe for setup. For CI, use a Postgres container with pgvector.

## Consequences

- sqlite-vec crate is deleted — it served its purpose as a prototype
- Agents with `vector_storage` need Postgres available (local or managed)
- Time travel works correctly for vector search — no accuracy trade-offs
- The ancestry-based filter is a native WHERE clause, not application post-filtering
- Future optimizations (materialized ancestry tables, partial indexes per timeline) are straightforward SQL
- Aligns with ADR 100: API surface scales with backend capability
