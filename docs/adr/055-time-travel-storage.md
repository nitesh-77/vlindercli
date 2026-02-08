# ADR 055: Time-Travel Storage

**Status:** Proposed

## Context

ADR 054 made `SubmissionId` = git commit SHA. Every `RequestMessage` (kv-get, kv-put, vector-store, etc.) now carries a SHA that represents a position in a causal graph. Storage engines receive this on every operation but ignore it — they overwrite state on each write.

This means storage is destructive. If an agent writes `/todos.json` on turn 3 then again on turn 5, the turn-3 state is gone. You can reconstruct the conversation from git, but you can't reconstruct the agent's data at any historical point.

The SHA flowing through every service call is a versioning key waiting to be used. If storage engines tag writes with the SHA, the entire system becomes a Merkle DAG: conversation history (git) + data state (storage) = complete snapshot at any turn. Time travel falls out for free.

## Problem Statement

**What storage contract makes time-travel work across any backend?**

Three things need answers:

### 1. Storage Trait Contract

Current write operations are destructive:
```
kv-put(path, content)           → overwrites
vector-store(key, vector, meta) → overwrites
```

Versioned writes would be append-only:
```
kv-put(path, content, submission)           → appends version
vector-store(key, vector, meta, submission) → appends version
```

Read operations need an `at` parameter:
```
kv-get(path)                    → latest (backwards compatible)
kv-get(path, at: sha)           → state at that turn
vector-search(query, at: sha)   → search against vectors as of that turn
```

Questions to resolve:
- Does `at` filter by exact SHA, or by "latest as of this SHA" (requires walking the commit parent chain)?
- Who resolves "latest as of"? The storage engine, or a coordinator that knows the git DAG?
- How does vector search work with versions? Filter by SHA in metadata? Maintain per-version indexes?
- Is the submission on writes implicit (from the RequestMessage) or explicit in the SDK?

### 2. Backend Implementation Strategy

The contract must work across fundamentally different storage engines:

**Relational (SQLite, Postgres):** Add a `sha` column. Writes become INSERT (not UPSERT). Reads add `WHERE sha = ?`. "Latest" is `ORDER BY rowid DESC LIMIT 1` or requires a HEAD pointer.

**Key-value (Redis, DynamoDB):** Composite key `{path}:{sha}`. Every version is a unique key. "Latest" needs an index or pointer.

**Vector stores (SQLite-vec, Qdrant, Pinecone):** Vectors tagged with SHA in metadata. Search filters by version. Per-version indexes may be needed for performance.

**Object stores (S3, MinIO):** Native versioning exists. SHA maps to version tag.

Questions to resolve:
- Is there a universal pattern, or does each backend need its own versioning strategy?
- How do we handle storage growth? Pruning/compaction strategy? GC of versions older than N turns?
- Does "latest" mean "most recent write to this path" or "write from the most recent ancestor in the git DAG"? The latter is correct but requires DAG awareness.

### 3. User Experience

Time travel is only useful if users can reach it. What does the UX look like?

**Inspection:**
```
vlinder inspect <sha>                    # show conversation + data state at this turn
vlinder inspect <sha> --storage          # dump all KV/vector state at this turn
vlinder diff <sha1> <sha2>              # what changed between two turns
```

**Replay:**
```
vlinder replay <sha>                     # re-run from this point with same inputs
vlinder replay <sha> --from-turn 3       # re-run from turn 3 onwards
```

**Branching:**
```
vlinder branch <sha>                     # fork conversation at this turn
vlinder run --session <sha>              # resume from a specific point
```

**Debugging:**
```
vlinder trace <sha>                      # show all service calls made during this turn
vlinder trace <sha> --kv                 # show KV reads/writes for this turn
```

Questions to resolve:
- Which of these are day-one vs future?
- Is `vlinder inspect` a read-only operation (query storage at SHA), or does it reconstruct state by replaying from the beginning?
- How does branching interact with storage? Fork the data too, or share it (copy-on-write)?
- Does the agent need to know it's being replayed, or is replay transparent?

## Constraints

- Backwards compatible: agents that don't use `at` get current behavior (latest)
- The SHA already flows through `RequestMessage.submission` — no protocol changes needed
- Storage growth must be bounded — infinite append-only is not viable for local-first
- Git is the source of truth for causal ordering — storage engines should not need to understand the DAG
- The SDK should make versioned reads opt-in, not mandatory

## Open Questions

1. **DAG awareness:** Should storage engines know about git parent chains, or should a coordinator resolve "state at SHA X" into concrete queries?
2. **Garbage collection:** What's the retention policy? Keep all versions? Last N turns? Prune on `clear`?
3. **Vector versioning:** Is per-version vector indexing practical, or should vectors be treated as append-only with metadata filtering?
4. **Cross-agent state:** If agent A writes at SHA X and agent B reads at SHA Y, how do versions interact across sessions?
5. **Performance:** What's the overhead of versioned reads/writes vs. current destructive operations?
