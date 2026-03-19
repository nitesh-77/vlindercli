# ADR 119: Dolt as Content-Addressed State Store

**Status:** Draft

## Context

The platform currently implements content-addressed storage by hand in SQLite (`vlinder-sqlite-kv`): values, snapshots, and state_commits are stored in custom tables that mimic git's object model. Branch operations (fork, promote) are implemented manually in the DAG store. This works but duplicates functionality that version-controlled databases provide natively.

Dolt (MySQL-compatible) and Doltgres (Postgres-compatible) are databases with git semantics built in. Every write can be a commit. Every state is addressable by hash. Branches, merges, and checkout-by-hash are SQL operations. The storage engine (Prolly trees) provides structural sharing and content-level deduplication automatically.

### Database connections are stateful

Current workers (KV, infer, embed, vector) are stateless — each request/response pair is independent. Database connections are inherently stateful: prepared statements, transaction context, temp tables, and SET variables accumulate across queries within a session.

The Dolt control plane (branch checkout, auto-commit configuration, hash retrieval) and data plane (agent SQL queries) share the same connection. The worker crate manages both on behalf of the agent.

## Decision

### 1. `vlinder-dolt` crate with Postgres-protocol provider host

A new crate `vlinder-dolt` implements the provider plugin contract (ADR 120) with a Postgres wire protocol listener, using the `postgres-protocol` crate (sfackler's low-level wire protocol crate from the rust-postgres ecosystem). This is analogous to how `vlinder-ollama` implements the contract with an HTTP listener.

The crate:
- Accepts Postgres wire protocol connections from agents
- Proxies data plane queries to Doltgres
- Injects control plane operations invisibly:
  - `@@dolt_transaction_commit=1` on connection setup (auto-commit)
  - `CALL DOLT_CHECKOUT('branch')` scoped to the session's current branch
  - `SELECT HASHOF('HEAD')` after each transaction to capture the state hash
- Emits request/response messages on the queue with captured wire bytes
- Reports the Dolt commit hash for the DAG node snapshot

The agent connects to `postgres.vlinder.local` — same virtual hostname pattern as HTTP provider hosts. The sidecar resolves it to itself and proxies to the real Doltgres. The agent sees a normal Postgres database. It does not know about Dolt.

### 2. Dolt provides native content-addressed storage

For agents that declare a database requirement, Dolt's native version control replaces the need for the hand-built values/snapshots/state_commits model. Dolt provides:
- Content-addressed storage (Prolly trees with structural sharing)
- Branching (`CALL DOLT_BRANCH`)
- Checkout by hash (`CALL DOLT_CHECKOUT`)
- Merge (`CALL DOLT_MERGE`)
- Commit history (`dolt_log`)
- Chunk-level deduplication (identical data across branches is stored once)

The platform's fork/promote operations map directly to Dolt branch operations instead of being implemented manually.

### 3. Connection-per-session worker lifecycle

Database workers maintain a persistent Postgres connection per agent session. The connection carries session state (branch checkout, transaction context) that persists across multiple request/response pairs. This is a new worker lifecycle — distinct from the stateless request-in/response-out model used by HTTP workers.

### 4. Agent declares its database in the manifest

The agent manifest declares database requirements, same as it declares model or storage requirements today. The connection string is the agent's configuration — different agents can use different Doltgres instances.

```toml
name = "inventory-agent"
database = "postgres://user:pass@doltgres.internal:5432/inventory"
```

The platform does not manage Doltgres. The agent (or its operator) brings their own. The `vlinder-dolt` crate connects to it, proxies the agent's queries, and manages the version control layer.

### 5. Vlinder operations map to Dolt operations

| Vlinder operation | Dolt operation |
|---|---|
| New session | `DOLT_BRANCH('session-xyz')` from main HEAD |
| Agent write | Auto-committed via `@@dolt_transaction_commit=1` |
| Capture state | `SELECT HASHOF('HEAD')` after each transaction |
| Fork session | `DOLT_BRANCH('fork-name')` + `DOLT_CHECKOUT('fork-name')` |
| Promote branch | `DOLT_CHECKOUT('main')` + `DOLT_MERGE('fork-name')` |
| Time travel | `DOLT_CHECKOUT(hash)` |
| Resume session | `DOLT_CHECKOUT('session-xyz')` |

All Dolt operations are SQL — they flow over the same Postgres wire protocol connection that the agent's data plane queries use.

### 6. KV worker remains for lightweight agents

Not every agent needs a full database. Agents that only need key-value storage continue to use the existing KV worker. Doltgres is for agents that declare a database requirement in their manifest.

### 7. State hash flows through the queue

The Dolt commit hash (`HASHOF('HEAD')`) is the state hash for the database instance. It must flow through the queue so the DAG projector can write it into the DAG node's `Snapshot` under the database `Instance` key. This is distinct from ADR 118's `payload_hash` (a content hash of the response bytes for payload storage). Both are entries in the Snapshot but serve different purposes: the payload hash enables content-addressed retrieval of service call bytes, the Dolt hash enables checkout of full database state. The mechanism for carrying the Dolt hash on the message is not yet resolved.

This follows CQRS: all writes go through the queue, the DAG node is a projection. The sidecar does not write to the DAG store directly.

## Consequences

- Agents interact with Doltgres as a normal Postgres database — zero Dolt awareness in agent code
- `vlinder-dolt` crate handles Dolt control plane transparently
- Content-addressed storage, branching, and deduplication are provided by Dolt natively — no hand-built implementation
- Time travel uses Dolt's native `CHECKOUT` by hash
- Fork and promote map to Dolt branch operations
- The provider host abstraction expands beyond HTTP to include Postgres wire protocol
- Workers gain a new lifecycle model: connection-per-session for stateful protocols
- `postgres-protocol` crate provides low-level wire protocol parsing without a high-level server or client framework
- Doltgres is a bring-your-own dependency — the agent operator manages the database, the platform connects to it
