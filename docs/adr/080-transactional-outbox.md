# ADR 080: Transactional Outbox — Synchronous DAG Recording

## Status

Accepted

## Context

ADR 079 established the State Service and identified a race condition: `read_latest_state` queries the DagStore at session startup, but the async dag-sqlite NATS consumer may not have processed the latest messages yet. The state returned can be stale.

The race is structural. No amount of retry or delay eliminates it — an async consumer is always eventually consistent. The fix is to make recording synchronous with message generation.

## Decision

### RecordingQueue decorator

A `RecordingQueue` wraps any `MessageQueue` and a `DagStore`. Every `send_*` call:

1. Clones the typed message
2. Converts to `ObservableMessage` via `From` impl
3. Looks up parent hash for Merkle chaining
4. Builds `DagNode` via the existing `build_dag_node()`
5. Inserts into `DagStore` synchronously
6. Forwards the original send to the inner queue

Receive and routing methods delegate straight through — recording only happens on the send side.

### Decorator, not inheritance

The decorator pattern was chosen over modifying `NatsQueue` or `InMemoryQueue` directly because:

- Recording is orthogonal to transport — it should work with any queue backend
- The existing queue implementations stay focused on their transport concerns
- It's easy to disable recording (just don't wrap)

### Merkle chain cache

The `RecordingQueue` maintains an in-memory `HashMap<String, String>` mapping session IDs to the hash of the last recorded node. This is a fast-path for consecutive sends within the same process (e.g., a container runtime worker sending multiple `RequestMessage`s in a row). On cache miss (first message in a session, or after process restart), it falls back to `DagStore::latest_node_hash()` — a new trait method that queries the shared store for the most recent node hash.

### Writes go through the State Service

The `RecordingQueue` writes to the gRPC State Service — the same shared store all workers use. Every worker, regardless of what machine it runs on, records into and reads from the same store. The Merkle chain stays consistent across all processes.

### State Service is required

`recording_from_config()` fails if the State Service is unreachable. Recording is not optional — silently skipping it would defeat the entire purpose of the outbox. If the State Service is down, workers should not start.

## Consequences

- Every `send_*` call records a DagNode before the message hits the wire — no async lag
- Queries against the DagStore always see the latest state
- The dag-sqlite NATS consumer is removed — the outbox replaces it
- `DagStore` trait gains `latest_node_hash()` — implemented in SQLite and gRPC
- Queue creation points (`agent run`, `fleet run`, workers) use `recording_from_config()` instead of `from_config()`
