# ADR 100: Storage Traits Are the Time Travel Contract

## Status

Draft

## Context

`vlinder timeline checkout` restores an agent's state from a previous
point in time. The system captures everything needed to reconstruct
that state:

| What | How it's captured |
|---|---|
| Agent files | ObjectStorage → StateCommit |
| Agent embeddings | VectorStorage → StateCommit |
| State snapshots | StateStore |
| Message flow | DagWorker (every observable message) |
| Inference calls + diagnostics | RequestMessage/ResponseMessage in DAG |
| Embedding calls + diagnostics | RequestMessage/ResponseMessage in DAG |
| Container diagnostics | CompleteMessage in DAG |

This is the full observable surface. Every side effect, every service
call, every diagnostic — all captured.

But none of this is represented in the domain as a unified concept.
ObjectStorage and VectorStorage are traits. DagWorker is a trait.
StateStore is a trait. The fact that they together form the time travel
contract is implicit — spread across implementations, not expressed as
a domain relationship.

### The transparent proxy illusion

Inference and embedding providers (Ollama, OpenRouter) are stateless
services. The platform acts as a transparent proxy — the agent sends
a real Ollama request to `ollama.vlinder.local`, gets a real Ollama
response back. The API surface is the provider's. The agent doesn't
know Vlinder exists.

Storage breaks this model. The platform adds something the underlying
backend doesn't have: time travel. Every write must be intercepted to
track state hashes, produce content-addressed snapshots, and enable
checkout. A raw proxy to SQLite, DynamoDB, or Postgres would bypass all
of this.

This means the storage API surface belongs to the platform, not the
backend. An agent can't use the full DynamoDB API or connect to a raw
Postgres socket — time travel requires the platform to control both
sides of the contract.

### The trade-off is explicit

The reduction in API surface is the cost of time travel. Time travel is
something DynamoDB and Postgres don't provide. What Vlinder provides is
exactly that, and in order to get it, some backend surface must be
constrained. What doesn't get lost is the backend's reliability and
scalability — those carry through.


## Decision

### Storage traits define the time travel boundary

The storage traits collectively define the time travel contract. What
they capture is what gets hashed (ADR 097), what the log shows
(ADR 099), and what gets restored on checkout (ADR 098).

The observable surface is:

- **Agent state**: ObjectStorage (files) + VectorStorage (embeddings),
  snapshotted as content-addressed StateCommits
- **Conversation state**: DagWorker, recording every observable message
  including all diagnostics from inference, embedding, and container
  runtime

Everything the system records is restorable. Everything restorable
is observable.

### Hostnames name the backend, not the capability

Storage backends follow the same provider hostname pattern as inference
(ADR 090). The hostname says what the backend is:

```
sqlite.vlinder.local          # KV backed by local SQLite
sqlite-vec.vlinder.local      # Vector backed by local sqlite-vec
dynamodb.vlinder.local        # KV backed by AWS DynamoDB
postgres.vlinder.local        # KV backed by PostgreSQL
foundationdb.vlinder.local    # KV backed by FoundationDB
```

The agent manifest declares the backend:

```toml
[requirements.services.kv]
provider = "sqlite"

[requirements.services.vec]
provider = "sqlite-vec"
```

Different backends, different hostnames, different crates. Swapping
SQLite for DynamoDB changes the manifest and the hostname. The request
and response types stay the same for the shared operations.

Like inference providers (ADR 090), the crate boundary is the billing
relationship. People choose DynamoDB or CosmosDB for managed
reliability, operational trust, and ecosystem fit. The hostname
communicates where your data lives. The API surface is the platform's;
the durability guarantees are the provider's.

### API surface scales with backend capability

Not all backends expose the same operations. The platform's contract
is the minimum needed for time travel. Backends that natively support
richer operations can expose more:

| Backend | KV surface | Why |
|---------|-----------|-----|
| SQLite | get, put, list, delete | Minimum viable — platform manages state externally |
| DynamoDB | get, put, list, delete, query | DynamoDB's native query doesn't break time travel |
| FoundationDB | full surface | Native MVCC and snapshot restore align with time travel |

| Backend | Vector surface | Why |
|---------|---------------|-----|
| sqlite-vec | store, search, delete | Minimum viable |
| Qdrant | store, search, delete, filter | Qdrant's filtering doesn't break time travel |

Each backend crate ships what's ready and what's compatible with time
travel. No promises about surfaces that don't exist yet. The hostname
tells the agent what it's working with.

### Each crate is a provider crate (ADR 090)

Storage crates follow the same structure as inference provider crates:

```
vlinder-sqlite              # KV: get, put, list, delete
vlinder-sqlite-vec          # Vector: store, search, delete
vlinder-dynamodb            # KV: get, put, list, delete, query
vlinder-foundationdb        # KV: full surface
```

Each crate declares its hostname, routes, and typed payloads. The
sidecar's provider server serves these hostnames alongside inference
providers on port 80.

### State tracking lives in the sidecar, not the agent

KV operations carry state hashes for time travel. The sidecar's
provider server manages the state cursor — it injects the current
state hash into requests and extracts the new hash from put responses.
The agent never sees state hashes. It calls `sqlite.vlinder.local/put`
with a key and value; the platform handles versioning.


## Consequences

- The storage traits carry more weight than "where to put files" — they
  define the boundary of time travel
- State not saved through these traits is not restorable and does not
  appear in the DAG
- New storage backends must follow the same contract: save it, hash it,
  restore it — but may expose additional operations
- The API surface is the platform's, not the backend's — no raw socket
  proxying, no wire protocol emulation
- Backend reliability and scalability carry through — SQLite for dev,
  DynamoDB/Postgres/FoundationDB for production
- Each backend is a separate crate with its own hostname, routes, and
  SDK dependency
- API surface grows with backend capability, not by promise
