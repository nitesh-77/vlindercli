# ADR 078: Typed Message Storage in Git DAG

## Status

Accepted (validated in 3bc694a, 05972b8)

## Context

The git DAG (ADR 064, 069, 070, 071) captures every protocol message as a commit. An earlier draft of this ADR proposed incremental enrichments: `message.toml`, `route.toml`, `state.toml`, stripping placeholders. But implementing those enrichments revealed a deeper problem: the pipeline itself is the bottleneck.

### The transformation pipeline is lossy

The current pipeline has four stages:

1. **Typed messages** — `InvokeMessage`, `RequestMessage`, `ResponseMessage`, `CompleteMessage`, `DelegateMessage`. Rich domain objects with every field the system knows: `harness`, `runtime`, `agent_id`, `service`, `backend`, `operation`, `sequence`, `correlation_id`, `reply_subject`, typed diagnostics.
2. **NATS wire format** — messages are decomposed into subject segments (`vlinder.<sub>.req.<agent>.<service>.<backend>.<op>.<seq>`), headers (`session-id`, `diagnostics`, `protocol-version`), and raw payload bytes.
3. **DagNode** — the DAG observer re-parses NATS subjects via `parse_from_to()`, flattening five distinct message shapes into one generic struct with `from: String` / `to: String`. Fields lost: `operation`, `sequence`, `runtime`, `correlation_id`, `reply_subject`, `harness` (as typed enum). Diagnostics become `Vec<u8>` (JSON bytes).
4. **Git tree files** — `DagNode` fields are hand-formatted into `message.toml`, diagnostics JSON is converted to TOML via `diagnostics_json_to_toml()` / `json_value_to_toml_value()`, stderr is extracted from JSON.

Each stage loses fidelity. By the time data reaches the git tree, it's a shadow of what the typed message contained. Incremental enrichments are patches on this fidelity loss — adding back information that was available at stage 1 but discarded at stage 3.

### The typed messages are already serializable

All five message types are clean data structs. Their diagnostics types already derive `Serialize` / `Deserialize`. The supporting types (`SessionId`, `SubmissionId`) already derive serde traits. Adding `#[derive(Serialize)]` to the message structs themselves is trivial.

The intermediate `DagNode` representation and all the string-parsing reconstruction code exist only because the DAG observer currently sits at the NATS layer (stage 2) instead of the domain layer (stage 1).

### Git is content-addressed — use it

Every blob in git is stored once, referenced by SHA. In a 20-message conversation:

- `session_id` is identical across all 20 messages — one blob, referenced 20 times
- `submission_id` is identical within a submission — one blob, referenced N times
- `protocol_version` is identical across ALL conversations on the same binary — one blob, forever
- `type` has only 5 possible values — at most 5 blobs across every conversation ever
- `from` / `to` have limited cardinality — a handful of blobs, heavily reused

Storing one large file per message defeats this. A single changed field (e.g., `created_at`) makes the entire blob unique, even though 90% of the content is shared with other messages. Splitting into one file per field turns git's content-addressing into automatic deduplication — no compression code needed.

### Optimize for projection, not human readability

The git tree is a data lake. Its consumers are tools and LLMs — agents that author other agents will use coding agents to munge the logs. The raw data should be lossless and easy to project programmatically. Human-readable views (`vlinder show`, `vlinder timeline`) are projections of the raw data, not the storage format itself.

## Decision

### Serialize typed messages directly, one file per field

The DagWorker receives `ObservableMessage` instead of raw NATS wire format. Each message field becomes its own file in the message directory. Scalar fields are plain-text files containing just the value. Binary fields (`payload`, `stderr`) are raw blobs. Structured fields (`diagnostics`) are serialized via serde.

#### InvokeMessage

```
20260211-143052.000-cli-invoke/
├── type                    # "invoke"
├── session_id              # "ses-abc123"
├── submission_id           # "sub-def456"
├── protocol_version        # "0.1.0"
├── created_at              # "2026-02-12T14:30:52.000Z"
├── harness                 # "cli"
├── runtime                 # "container"
├── agent_id                # "http://127.0.0.1:9000/agents/support-agent"
├── state                   # "abc123def456" (only when present)
├── payload                 # raw bytes
└── diagnostics.toml        # InvokeDiagnostics via serde
```

#### RequestMessage

```
20260211-143053.000-support-agent-request/
├── type                    # "request"
├── session_id              # "ses-abc123"  ← same blob
├── submission_id           # "sub-def456"  ← same blob
├── protocol_version        # "0.1.0"       ← same blob
├── created_at              # unique
├── agent_id                # same blob if same agent
├── service                 # "infer"
├── backend                 # "ollama"
├── operation               # "chat"
├── sequence                # "1"
├── state                   # (only when present)
├── payload                 # unique
└── diagnostics.toml        # RequestDiagnostics via serde
```

#### ResponseMessage

```
20260211-143054.000-infer.ollama-response/
├── type                    # "response"
├── session_id              # "ses-abc123"  ← same blob
├── submission_id           # "sub-def456"  ← same blob
├── protocol_version        # "0.1.0"       ← same blob
├── created_at              # unique
├── agent_id                # same blob if same agent
├── service                 # "infer"       ← same blob as request
├── backend                 # "ollama"      ← same blob as request
├── operation               # "chat"        ← same blob as request
├── sequence                # "1"           ← same blob as request
├── correlation_id          # links to originating request
├── state                   # (only when present)
├── payload                 # unique
└── diagnostics.toml        # ServiceDiagnostics via serde
```

#### CompleteMessage

```
20260211-143055.000-support-agent-complete/
├── type                    # "complete"
├── session_id              # "ses-abc123"  ← same blob
├── submission_id           # "sub-def456"  ← same blob
├── protocol_version        # "0.1.0"       ← same blob
├── created_at              # unique
├── agent_id                # same blob
├── harness                 # "cli"         ← same blob as invoke
├── state                   # (only when present)
├── payload                 # unique
├── diagnostics.toml        # ContainerDiagnostics via serde (minus stderr)
└── stderr                  # raw bytes (only when present)
```

#### DelegateMessage

```
20260211-143056.000-coordinator-delegate/
├── type                    # "delegate"
├── session_id              # "ses-abc123"  ← same blob
├── submission_id           # "sub-def456"  ← same blob
├── protocol_version        # "0.1.0"       ← same blob
├── created_at              # unique
├── caller_agent            # "coordinator"
├── target_agent            # "summarizer"
├── reply_subject           # preserved — was lost by DagNode
├── state                   # (only when present)
├── payload                 # unique
├── diagnostics.toml        # DelegateDiagnostics via serde (minus stderr)
└── stderr                  # raw bytes (only when present)
```

File presence/absence is itself information — `service` only exists in Request/Response directories, `harness` only in Invoke/Complete, `stderr` only when the container produced output.

### DagNode goes away

`ObservableMessage` replaces `DagNode` as the input to the git DAG pipeline. The following code is deleted:

- **`DagNode` struct** — the typed messages are the representation
- **`parse_from_to()`** — typed messages already have routing fields
- **`diagnostics_json_to_toml()` / `json_value_to_toml_value()`** — serialize directly via serde
- **`extract_stderr()`** — stderr is a field on `ContainerDiagnostics`
- **`hash_dag_node()`** — replaced by hashing the serialized message, or use git's own tree hash
- **`MessageType` enum in `dag.rs`** — redundant with `ObservableMessage` variants
- **NATS subject parsing in `process_message()`** — the DagWorker receives typed messages, not wire format

### Typed messages gain Serialize

`InvokeMessage`, `RequestMessage`, `ResponseMessage`, `CompleteMessage`, `DelegateMessage` add `#[derive(Serialize)]`. Supporting types that lack it (`MessageId`, `HarnessType`, `Sequence`, `RuntimeType`) gain serde derives.

### Two independent NATS consumers replace DagCaptureWorker

The old `DagCaptureWorker` was an in-process fan-out dispatcher with an array of workers. This re-implemented what NATS gives natively — multiple consumers on the same stream, each independently receiving every message.

Two independent JetStream consumers subscribe to `vlinder.>`:

- **`dag-sqlite`**: Reconstructs `ObservableMessage` from NATS headers, converts to `DagNode`, inserts into SQLite. Owns its own Merkle chaining state (`HashMap<String, String>`).
- **`dag-git`**: Reconstructs `ObservableMessage` from NATS headers, calls `GitDagWorker::on_observable_message()`. Owns its own commit chain state.

Each consumer is an independent process with no shared dispatcher. NATS handles the multiplexing.

The shared `reconstruct_observable_message()` function reads NATS subject + headers + payload to reconstruct typed messages — the inverse of `NatsQueue::send_*()`.

#### Singletons as a sensible default

DAG workers are singletons — not a fundamental limitation, but the correct default for the current workload:

- **Git**: The bottleneck is the ref lock file (one per branch) and the sequential commit chain. Scaling option: ref-per-session parallelism — each session gets its own branch, workers update independent refs concurrently. Object store (blobs, trees, commits) is concurrent-safe. Trade-off: lose single-directory browsability.
- **SQLite**: Per-session Merkle chains are already independent in the HashMap — partitioning sessions across workers is mechanically straightforward.
- **NATS routing**: Requires session-id in the subject format (currently only in headers) so consumers can filter by session. Subject format is ours to change.

This is a workload-shape + UX decision, not an architecture decision. Current default (single branch, single HEAD, browse everything with `ls`) is correct for single-user moderate-throughput. When throughput demands it, the knobs exist — own ADR when needed.

### Earlier draft decisions are subsumed or simplified

| Earlier draft decision | Status |
|---|---|
| `message.toml` per directory | **Subsumed.** Every field is its own file — strictly more information than a hand-formatted summary. |
| Strip stderr from diagnostics.toml | **Preserved.** `diagnostics.toml` is serde-serialized with `#[serde(skip)]` on stderr. Raw `stderr` is a separate blob. |
| Omit uncaptured fields | **Preserved.** `#[serde(skip_serializing_if)]` on diagnostics types. |
| Materialize `state.toml` | **Deferred.** Orthogonal — state store access is independent of message serialization. |
| Session-level `route.toml` | **Deferred.** A projection of the raw data. Can be built from the per-field files. |
| Real Podman metadata | **Unchanged.** Still a wiring task in the container runtime. |
| Token counts from backends | **Unchanged.** Still a wiring task in inference workers. |
| Structured agent envelope | **Unchanged.** Still a contract evolution task. |
| Session duration | **Deferred.** Derivable from `created_at` files: first and last message. |
| Diff-friendly state keys | **Deferred.** Falls out from state materialization. |

## Consequences

- **Code removed**: DagNode, parse_from_to, JSON-to-TOML conversion, extract_stderr, NATS subject parsing for DAG — roughly 150 lines of reconstruction code replaced by direct serialization
- **Code added**: `#[derive(Serialize)]` on message types, `build_message_subtree()` rewritten to iterate typed fields
- **Full fidelity**: operation, sequence, correlation_id, reply_subject, runtime, harness — all preserved. Nothing is lost in translation
- **Storage efficiency**: content-addressing deduplicates stable fields across messages and across conversations. One-file-per-field maximizes blob reuse
- **Simpler pipeline**: typed message → serialize fields → git blobs. One stage, no intermediate representation
- **Projection-friendly**: every value independently addressable via `git show HEAD:<dir>/<field>`. LLMs and tools project the raw data however they need
- **DagStore (SQLite) unaffected**: this ADR only changes the git tree format. The SQLite DagStore still uses its own schema. Unifying them is a future concern
- **Breaking change**: existing git conversation repos have the old tree layout. New conversations use the new format. Migration is out of scope — the conversation store is ephemeral debugging data, not archival
