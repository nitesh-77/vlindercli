# ADR 072: Storage Snapshot Contract

## Status

Draft

## Context

ADR 055 introduced versioned state for time-travel debugging. The current implementation intercepts every kv-get and kv-put through the ServiceRouter, injects state hashes into request payloads, and maintains a parallel StateStore (SQLite) alongside the ObjectStorage.

This creates several problems:

1. **Tight coupling.** The ServiceRouter must understand kv payloads to inject/extract state hashes. The ObjectServiceWorker must maintain two stores (ObjectStorage + StateStore) in sync.

2. **Empty-state edge case.** When no state commit exists yet, the ServiceRouter injects an empty string hash, which the versioned get path doesn't handle — causing silent failures.

3. **Dangling pointers.** The state pointer (`~/.vlinder/state/<agent>.latest`) can reference a state commit in a deleted StateStore, breaking all subsequent kv reads.

4. **BYOS is blocked.** If an agent brings its own storage (S3, Postgres, etc.), the platform can't intercept writes to build the state chain. Direct agent-to-store communication bypasses the bridge entirely.

### The insight

The platform doesn't need to intercept every read and write. It only needs two things from the storage backend:

- **Snapshot**: an opaque hash identifying the current state
- **Restore**: roll back to a previously snapshotted state

This is how git works. Git doesn't intercept file writes. You work, then it snapshots. The SHA is the handle.

## Decision

### 1. Storage snapshot contract

Storage backends implement a two-method contract:

```rust
trait Snapshottable {
    /// Return an opaque identifier for the current state.
    fn snapshot(&self) -> Result<String, String>;

    /// Restore to a previously snapshotted state.
    fn restore(&self, hash: &str) -> Result<(), String>;
}
```

The hash is opaque to the platform. A SQLite backend might return a content-addressed hash of all key-value pairs. A Postgres backend might return a transaction ID. An S3 backend might return a version marker. The platform doesn't interpret it — just stores and replays it.

### 2. Snapshot is part of the invoke response

The agent includes its storage state hash in the `/invoke` response:

```json
{
  "response": "Here are your todos...",
  "stderr": "INFO: loaded model",
  "state": "abc123"
}
```

The agent's storage SDK calls `snapshot()` on its backend before returning. The runtime extracts the hash — same path as stderr extraction (ADR 071). No extra round trip, no write interception.

### 3. Restore at fork time

When forking at a previous point in the conversation:

1. Platform reads the state hash from the fork-point commit
2. Platform calls `restore(hash)` on the storage backend
3. Agent re-runs, talks directly to its store, sees correct state

### 4. Platform-provided storage implements the contract

The default SQLite storage provided by the platform implements `Snapshottable` internally. Agents using platform-provided storage get time travel for free — the platform manages both the storage and the snapshot chain.

### 5. What gets removed

- `ServiceRouter::inject_state()` / `extract_state()` — no more state hash injection into kv payloads
- `GetRequest.state` / `PutRequest.state` fields — kv requests become stateless again
- `ObjectServiceWorker::versioned_get()` / `versioned_put()` — versioning moves into the backend
- `~/.vlinder/state/<agent>.latest` — the state hash lives in the conversation store

## Consequences

- **BYOS with time travel**: any storage backend that implements `snapshot()`/`restore()` gets time travel. The platform doesn't need to understand the storage format.
- **No write interception**: agents talk directly to their storage for reads and writes. The bridge only handles service calls (infer, embed) that route through platform workers.
- **Simpler ObjectServiceWorker**: back to pure kv operations, no versioned paths.
- **Simpler ServiceRouter**: no state injection/extraction, no kv-specific payload manipulation.
- **Agent contract evolution**: the `/invoke` response must include a `state` field. This aligns with the structured response planned in ADR 071 (response + stderr + state).
- **Migration**: existing StateStore data becomes the initial implementation of `Snapshottable` for SQLite. The state chain in the conversation store replaces `~/.vlinder/state/*.latest`.

### Deferred

1. **Bridge-mediated vs direct storage access** — With this contract, agents *could* talk directly to their storage. But platform-provided storage still goes through the bridge (for now) because the bridge provides agent isolation (per-agent storage namespacing). Direct access is a future concern for BYOS agents.

2. **Snapshot granularity** — Currently one snapshot per turn. Sub-turn snapshots (after each kv-put) would enable finer-grained forking but add complexity. Defer until there's a real use case.
