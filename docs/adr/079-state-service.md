# ADR 079: State Service

## Status

Accepted (validated in 26eb0ac)

## Context

Time travel breaks on read-only turns. When an agent turn only reads state (e.g., "list my todos" via `kv_get`), no state hash propagates through the system. `ResponseMessage.state` is `None` for every service except `kv_put`. The `CompleteMessage` for that turn carries no state, the git commit gets an empty `State:` trailer, and `read_latest_state` finds nothing.

Result: `timeline checkout <sha>` on a read-only turn resumes the agent with no state — it runs against current storage instead of the storage at that point in time.

The root cause is twofold:

1. **State is only produced on writes.** Read-only service responses don't echo the state they received. A read-only turn should carry forward the same state hash — nothing changed.

2. **State recording is async and decoupled from state reading.** Messages flow through NATS to a dag-sqlite consumer that writes to SQLite. `read_latest_state` shells out to git. Two different stores, both eventually consistent, with no guarantees about ordering or freshness.

The current `read_latest_state` in the harness shells out to `git -C` to grep commit messages for `State:` trailers. This collocates the harness with the git repository on the local filesystem. Replacing git with direct SQLite access would just trade one colocation problem for another.

## Decision

### 1. Echo state on every service response

Service workers set `response.state = request.state.clone()` on every response. Read operations echo the state they received (nothing changed). Write operations (`kv_put`) continue to produce new state as they do today. The state hash now flows through every turn, not just writes.

### 2. State Service — gRPC wrapper around state storage

A new gRPC service, the **State Service**, owns all state persistence. Same pattern as the Registry Service (ADR 043): domain trait → SQLite implementation → gRPC server → gRPC client that implements the same trait.

The existing `DagStore` trait gains one method:

```rust
fn latest_state(&self, agent_name: &str) -> Result<Option<String>, String>;
```

The State Service exposes this (and the existing `insert_node`, `get_node`, `get_session_nodes`) over gRPC. The harness calls the State Service to read state — no git, no local filesystem, no subprocess.

### 3. Transactional outbox — record at message generation time

Messages are recorded to the State Service synchronously at the point of generation, not asynchronously through a NATS consumer. This eliminates the race between "query latest state" and "has the consumer caught up?" The DB is always current.

The dag-sqlite NATS consumer becomes redundant once the outbox is in place.

### 4. `vlinder timeline` owns git mutations

Git's UX for time travel (checkout, log, diff, branch) is too good to reimplement. `vlinder timeline` continues to shell out to git for UX. But:

- All git mutations go through `vlinder timeline` (enforced by convention)
- `vlinder timeline` syncs the DB on every git mutation
- The DB is the source of truth for state; git is a UX layer for browsing

## Risks

### SQL backend

The current implementation uses SQLite. SQLite is fine for single-user local mode, but won't scale to production multi-agent workloads. The fix is already in the architecture: the gRPC service boundary means the storage backend is a configuration choice. Swap SQLite for Postgres (pgbouncer + multi-node cluster) without changing any client code. We decided on SQL, not SQLite — the implementation is provisional.

### Conversations repo

Users should not mutate the git conversations repo directly. `vlinder timeline` is the only supported path. If you bypass it and corrupt state, that's on you. The DB is the source of truth; git is a derived view maintained by `vlinder timeline`.

### Git colocation

`vlinder timeline` currently shells out to local git via `git -C <path>`, which collocates the CLI with the git repo. Near-term fix: configure a remote git URL (`git://` or `ssh://`) in config, so timeline commands work from any machine. The git client and server are decoupled — same git UX, no filesystem dependency.

## Consequences

- `read_latest_state` becomes a State Service gRPC call — harness is fully decoupled from git and filesystem
- Every turn carries state, not just write turns — time travel works on read-only turns
- State is durable the instant it's produced — no NATS consumer lag, no ordering races
- The dag-git NATS consumer can stay (git is still useful for browsing) or move behind `vlinder timeline`
- The State Service is the data-plane equivalent of the Registry Service — same gRPC pattern, same local/distributed deployment model
- `vlinder timeline` becomes the single gateway for git mutations, keeping git and DB in sync
- Whoever generates a message records it — no centralized consumer, no async lag
- The Merkle chain is maintained by the State Service internally — callers send data, the service computes hashes
