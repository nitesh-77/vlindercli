# ADR 013: AgentFS as Specification (MIT-Only Storage)

## Status

Superseded

The specific AgentFS schema is now one implementation of the `ObjectStorage` trait. The domain defines the abstract interface; implementations include `SqliteObjectStorage` (this ADR's approach) and `InMemoryObjectStorage` (for testing). See `src/domain/storage.rs` for the trait, ADR 018 for the queue-based access pattern.

## Context

Agents need persistent file storage for caching, state, and data. The original plan was to use AgentFS SDK, but this pulled in Turso dependencies that include GPL-licensed code (bloom crate via turso_core → DiskANN).

The project requires 100% MIT/Apache-2.0 licensing to remain permissively licensed.

## Decision

**Treat AgentFS as an Open Specification, not an SDK dependency.**

Instead of importing agentfs-sdk, we implement the specification directly:

```sql
CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY,
    content BLOB NOT NULL,
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch())
);
```

**MIT-only storage stack:**

```toml
# Cargo.toml
rusqlite = { version = "0.32", features = ["bundled"] }
```

- `rusqlite` with `bundled` feature embeds SQLite (MIT/public domain)
- WAL mode for concurrency
- Sync API (simpler than async, matches our usage pattern)

**Host functions exposed to WASM agents:**

```rust
fn put_file(path: String, content: Vec<u8>) -> String;   // "ok" or error
fn get_file(path: String) -> Vec<u8>;                    // content or error
fn delete_file(path: String) -> String;                  // "ok"/"not_found"
fn list_files(dir: String) -> String;                    // JSON array of paths
```

**Single .db file per agent:**

```
.vlinder/agents/{name}/{name}.db
```

## Consequences

- 100% MIT/Apache-2.0 license compliance
- No external SDK dependency to track or update
- AgentFS-compatible schema for future interoperability
- Agents get persistent storage for caching and state
- Each agent's storage is isolated in its own database file
