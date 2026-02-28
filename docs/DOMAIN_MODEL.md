# Domain Model

Core types and abstractions in VlinderCLI. Separates the **domain** (abstract protocol in `vlinder-core`) from **implementations** (concrete infrastructure in leaf crates).

For architectural decisions, see `docs/adr/`. For the vision, see `VISION.md`.

---

## Architecture Overview

Queue-based message-passing architecture (ADR 018). Everything is a service that consumes messages and produces responses:

```
                         Message Queue (NATS)

   +-----------+ +-----------+ +-----------+ +------------+
   | inference | | embedding | |  storage  | |   agents   |
   |  workers  | |  workers  | |  workers  | |(containers)|
   +-----------+ +-----------+ +-----------+ +------------+
   +-----------+ +-----------+ +-----------+ +------------+
   |   state   | |    DAG    | |  catalog  | |   secret   |
   |  worker   | |  worker   | |  worker   | |   worker   |
   +-----------+ +-----------+ +-----------+ +------------+
        ^             ^             ^              ^
        +-------------+-------------+--------------+
                       all just workers
```

`vlinder daemon` runs a **Supervisor** that spawns worker processes. Workers connect to **NATS** for messaging and **gRPC** for registry/state/secret/catalog. There is no in-process mode -- all execution flows through the queue (ADR 062).

In-memory implementations (`InMemoryQueue`, `InMemoryRegistry`, etc.) exist only as unit test doubles.

---

## Crate Layout

```
vlinder-core           Domain types, traits, and protocol specification
vlinder-nats           NATS MessageQueue implementation
vlinder-sql-registry   PersistentRegistry + gRPC service + SQLite storage
vlinder-sql-state      SqliteDagStore + gRPC state service
vlinder-git-dag        Git-based DAG worker (conversation commits)
vlinder-ollama         Ollama inference/embedding workers + catalog
vlinder-infer-openrouter  OpenRouter inference worker + catalog
vlinder-sqlite-kv      SQLite object storage worker
vlinder-sqlite-vec     SQLite-vec vector storage worker
vlinder-podman-runtime Container runtime (Podman)
vlinder-catalog        Catalog gRPC service
vlinder-harness        Harness gRPC service (server + client)
vlinder-sidecar        Agent sidecar container
vlinder                CLI binary (vlinder)
vlinderd               Daemon binary (vlinderd) + config + factories + supervisor
```

---

# Part 1: Domain (Abstract Protocol)

The domain defines **what** the system does: types, traits, and contracts. Infrastructure-agnostic.

Location: `crates/vlinder-core/src/domain/`

---

## Identity Types

### ResourceId

URI-based resource identity. Key for looking up any resource in the registry.

- Scheme indicates resource type: `sqlite://`, `ollama://`, `openrouter://`, `memory://`, `http://`
- Supports parsing into `scheme()`, `authority()`, and `path()` components
- Location: `domain/resource_id.rs`

### MessageId, SubmissionId, SessionId, TimelineId

Tracking identifiers for message flow:

- `MessageId` -- unique per message (UUID)
- `SubmissionId` -- groups messages for one user request. Content-addressed SHA-256 hash of (payload, session_id, parent_submission) per ADR 081
- `SessionId` -- groups submissions into a conversation. Format: `ses-{uuid}`
- `TimelineId` -- branch-scoped identity for time-travel (ADR 093)

### Sequence / SequenceCounter

Ordering for service requests within a submission. Starts at 1, increments per request. Thread-safe counter allocates sequential values.

### ImageRef, ImageDigest

OCI container identity types with parse-time validation:

- `ImageRef` -- validated OCI image reference (e.g., `localhost/echo-agent:latest`)
- `ImageDigest` -- validated content-addressed digest (e.g., `sha256:a80c4f17...`)
- Location: `domain/image_ref.rs`, `domain/image_digest.rs`

### ContainerId, PodId

Podman runtime identity newtypes for container and pod IDs.

---

## Agent & Fleet Types

### Agent

A unit of behavior with declared requirements: identity, executable (OCI image), models, services, mounts, prompts.

- Location: `domain/agent.rs`
- Key fields: `name`, `id: ResourceId`, `runtime: RuntimeType`, `executable: String`, `image_digest: Option<ImageDigest>`, `requirements: Requirements`, `object_storage`, `vector_storage`, `public_key`

### AgentManifest

Raw manifest from `agent.toml`. Deserialized separately from the resolved `Agent` type.

- Location: `domain/agent_manifest.rs`
- Contains: `RequirementsConfig`, `ServiceConfig`, `Protocol`, `MountConfig`, `PromptsConfig`

### Requirements / Prompts

What an agent needs to run (models, services, mounts) and optional prompt overrides.

### Model

An inference or embedding capability: name, type, provider, model path, content digest.

- Location: `domain/model.rs`
- `ModelType`: `Inference` | `Embedding`
- `Provider`: `Ollama` | `OpenRouter`

### ModelManifest

Raw manifest from `<model>.toml`. Separate from resolved `Model` type.

- Location: `domain/model_manifest.rs`

### Fleet / FleetManifest

A composition boundary for agents: name, entry-point agent, agents map.

- Location: `domain/fleet.rs`, `domain/fleet_manifest.rs`

### Storage Configuration

- `ObjectStorageType`: `Sqlite` | `InMemory` (test-only)
- `VectorStorageType`: `SqliteVec` | `InMemory` (test-only)
- Location: `domain/storage.rs`

---

## Protocol Types

Five typed messages flow through the queue (ADR 018, 044). Every message carries `protocol_version`, `submission`, and `session` for traceability.

### Message Flow

```
Harness --InvokeMessage--> Runtime --RequestMessage--> Service
                                   <--ResponseMessage--
        <--CompleteMessage--

Agent --DelegateMessage--> Agent (via runtime)
```

### InvokeMessage

Harness -> Runtime. Starts a submission. Carries `agent_id`, `harness: HarnessType`, `runtime: RuntimeType`, `state: Option<String>`, `timeline: TimelineId`, and `InvokeDiagnostics`.

### RequestMessage

Runtime -> Service. Agent requests a service operation (kv, vec, infer, embed). Carries `service`, `backend`, `operation`, `sequence`, and `RequestDiagnostics`.

### ResponseMessage

Service -> Runtime. Service replies with result. Echoes all request dimensions plus `correlation_id`. Carries `ServiceDiagnostics`.

### CompleteMessage

Runtime -> Harness. Submission finished. Carries final `state` hash and `ContainerDiagnostics`.

### DelegateMessage

Agent -> Agent (via runtime). One agent invoking another. Carries `caller_agent`, `target_agent`, `reply_subject`, and `DelegateDiagnostics`.

Location: `domain/message/` (one file per message type)

### ExpectsReply Trait

Type-level enforcement of request-response pairing. Terminal messages (`CompleteMessage`, `ResponseMessage`) do NOT implement it -- attempting to reply is a compile error.

### ObservableMessage

Unified enum wrapping all five message types for polymorphic handling (e.g., DAG recording).

### HarnessType

Entry point variants: `Cli`, `Web`, `Api`, `Whatsapp`. Routes completion messages back to the correct harness.

### MessageQueue Trait

Typed send/receive methods for all five message types, plus routing helpers and delegation support.

- Location: `domain/message_queue.rs`

---

## Routing

### RoutingKey

Determines NATS subject for message delivery. Variants for each message type (Invoke, Complete, Request, Response, Delegate, DelegateReply).

### AgentId, Nonce, ServiceBackend

Supporting types for routing: agent identity in routing context, one-shot uniqueness values, service-backend pairs.

### InferenceBackendType, EmbeddingBackendType

Backend selectors for routing inference/embedding requests to the right worker.

- Location: `domain/routing_key.rs`

---

## Time Travel Types

### DagNode / DagStore Trait

Merkle DAG for runtime tracing (ADR 067). One `DagNode` per NATS message.

- `hash`: `SHA-256(payload || parent_hash || message_type || diagnostics)` -- Merkle chain
- `parent_hash`: previous node in the session
- `message_type`: `Invoke` | `Request` | `Response` | `Complete` | `Delegate`
- `from` / `to`, `session_id`, `submission_id`, `payload`, `diagnostics`, `stderr`, `state`

`DagStore` trait: `insert_node()`, `get_node()`, `get_session_nodes()`, `get_children()`, `latest_state()`, `latest_node_hash()`.

### DagWorker Trait

Processes observable messages and persists them to the DAG.

### Timeline / SessionSummary

Timeline represents a named branch. SessionSummary provides summary data for the session viewer.

### Session / HistoryEntry

Conversation state for multi-turn interactions (ADR 054). Tracks conversation history, builds enriched payloads, manages submission chaining.

- Location: `domain/session.rs`

---

## Diagnostics

Each message type carries diagnostics specific to its emitter (ADR 071):

| Message | Diagnostics type | Emitter |
|---------|------------------|---------|
| Invoke | `InvokeDiagnostics` | Harness |
| Request | `RequestDiagnostics` | Container runtime |
| Response | `ServiceDiagnostics` + `ServiceMetrics` | Service workers |
| Complete | `ContainerDiagnostics` + `ContainerRuntimeInfo` | Container runtime |
| Delegate | `DelegateDiagnostics` | Container runtime |

`ServiceMetrics` is a tagged enum: `Inference { tokens_input, tokens_output, model }`, `Embedding { dimensions, model }`, `Storage { operation, bytes_transferred }`.

- Location: `domain/diagnostics.rs`

---

## Control Plane

### Registry Trait

Source of truth for all system state. Stores agents, models, jobs, and tracks capabilities.

- Agent operations: `register_agent()`, `register_manifest()`, `get_agent()`, `get_agents()`, `select_runtime()`
- Model operations: `register_model()`, `get_model()`, `delete_model()`
- Job operations: `create_job()`, `get_job()`, `update_job_status()`, `pending_jobs()`
- Capability registration for runtimes, storage types, inference/embedding engines
- Location: `domain/registry.rs`

### RegistryRepository Trait

Persistence layer for Registry state (save/load agents and models to durable storage).

- Location: `domain/registry_repository.rs`

### Job / JobId / JobStatus

Submitted job and current state.

- `JobId`: format `<registry_id>/jobs/<uuid>`
- `JobStatus`: `Pending` -> `Running` -> `Completed(String)` | `Failed(String)`

### Harness Trait + CoreHarness

API surface for agent interaction. `CoreHarness` is the canonical implementation:

- Session management with conversation history
- Content-addressed submission chaining (ADR 081)
- State tracking with pending/committed promotion (ADR 055)
- Timeline-scoped invocations with seal enforcement (ADR 093)
- Job creation and status tracking via the registry

- Location: `domain/harness.rs`

### Runtime Trait / RuntimeType

Agent execution protocol. The runtime polls input queues, executes agent code, and manages the request-response lifecycle.

- `RuntimeType`: `Container` (OCI containers via Podman)
- Methods: `id()`, `runtime_type()`, `tick()`, `shutdown()`
- Location: `domain/runtime.rs`

### ModelCatalog / CatalogService Traits

Resolves model names to Model configurations. `CompositeCatalog` dispatches across multiple backends.

- Location: `domain/catalog.rs`

### SecretStore Trait

Named secret storage (byte blobs). Used for agent identity keys (ADR 083).

- Location: `domain/secret_store.rs`

### AgentIdentity

Ed25519 key pair for agent identity. Provisioned at registration time.

- Location: `domain/identity.rs`

### Provider

Model provider enum: `Ollama` | `OpenRouter`. Includes HTTP routing information (`ProviderRoute`, `ProviderHost`).

- Location: `domain/provider.rs`

### Operation / ServiceType

`Operation`: verb being performed (Get, Put, List, Delete, Store, Search, Run, Chat, Generate).
`ServiceType`: platform service category (Kv, Vec, Infer, Embed).

---

## Queue Implementations (vlinder-core)

Test doubles and decorators that live alongside the domain:

| Type | Description |
|------|-------------|
| `InMemoryQueue` | VecDeque per subject -- test double only |
| `RecordingQueue` | Decorator -- records DAG nodes before forwarding |

Location: `crates/vlinder-core/src/queue/`

---

## Workers (vlinder-core)

DAG message reconstruction utilities for building `DagNode`s from raw NATS messages.

Location: `domain/workers/dag.rs`

---

## Path Types

Enforce absolute paths at compile time. Relative paths in manifests resolve against the manifest's directory at load time.

- `AbsoluteUri` -- absolute `file://` URI
- `AbsolutePath` -- absolute filesystem path
- Location: `domain/path.rs`

---

# Part 2: Implementations (Concrete Infrastructure)

Each leaf crate implements one or more domain traits.

---

## Registry (`vlinder-sql-registry`)

| Type | Role | Description |
|------|------|-------------|
| `PersistentRegistry` | `Registry` impl | Write-through to SQLite via `SqliteRegistryRepository` |
| `SqliteRegistryRepository` | `RegistryRepository` impl | SQLite persistence for agents and models |
| `RegistryServiceServer` | gRPC server | Wraps a `Registry`, serves gRPC |
| `GrpcRegistryClient` | gRPC client | Implements `Registry` trait via gRPC |
| `RegistryConfig` | Config | Engine availability (inference/embedding providers) |

## State (`vlinder-sql-state`)

| Type | Role | Description |
|------|------|-------------|
| `SqliteDagStore` | `DagStore` impl | SQLite-backed Merkle DAG persistence |
| `StateServiceServer` | gRPC server | Wraps a `DagStore`, serves gRPC |
| `GrpcStateClient` | gRPC client | Implements `DagStore` trait via gRPC |
| `SessionServer` | HTTP server | Session viewer for debugging |

## DAG (`vlinder-git-dag`)

| Type | Role | Description |
|------|------|-------------|
| `GitDagWorker` | `DagWorker` impl | Writes conversation commits to a git repo |

## Queue (`vlinder-nats`)

| Type | Role | Description |
|------|------|-------------|
| `NatsQueue` | `MessageQueue` impl | NATS JetStream with sync facade over async tokio |
| `NatsSecretStore` | `SecretStore` impl | NATS KV-backed secret storage |

## Container Runtime (`vlinder-podman-runtime`)

| Type | Role | Description |
|------|------|-------------|
| `ContainerRuntime` | `Runtime` impl | Executes OCI agents via Podman pods |

Tick-loop orchestrator: polls invoke/delegate queues, dispatches to containers via HTTP POST `/handle`, manages the request-response lifecycle.

## Inference (`vlinder-ollama`, `vlinder-infer-openrouter`)

| Type | Crate | Description |
|------|-------|-------------|
| `OllamaWorker` | `vlinder-ollama` | Inference + embedding via Ollama HTTP API |
| `OllamaCatalog` | `vlinder-ollama` | Model resolution from local Ollama |
| `OpenRouterWorker` | `vlinder-infer-openrouter` | Inference via OpenRouter cloud API |
| `OpenRouterCatalog` | `vlinder-infer-openrouter` | Model resolution from OpenRouter |

## Storage (`vlinder-sqlite-kv`, `vlinder-sqlite-vec`)

| Type | Crate | Description |
|------|-------|-------------|
| `KvWorker` | `vlinder-sqlite-kv` | SQLite-backed object storage worker |
| `SqliteVecWorker` | `vlinder-sqlite-vec` | sqlite-vec vector storage worker |

## Catalog (`vlinder-catalog`)

| Type | Role | Description |
|------|------|-------------|
| `CatalogServiceServer` | gRPC server | Wraps `CompositeCatalog`, serves gRPC |

## Harness gRPC Service (`vlinder-harness`)

| Type | Role | Description |
|------|------|-------------|
| `HarnessServiceServer` | gRPC server | Wraps a `Harness`, serves gRPC |
| `GrpcHarnessClient` | gRPC client | Implements `Harness` trait via gRPC |

---

# Part 3: Daemon Wiring (`vlinderd`)

The daemon is not domain -- it's the composition root that wires everything together.

## Supervisor

Process manager for `vlinder daemon`. Spawns worker processes per `Config`, waits for readiness (registry first, then state service), manages graceful shutdown.

## Factories

Config-to-implementation wiring. Each takes `&Config` and returns a trait object:

| Factory | Returns | Production impl |
|---------|---------|-----------------|
| `queue_factory` | `Arc<dyn MessageQueue>` | `NatsQueue` / `RecordingQueue` |
| `registry_factory` | `Arc<dyn Registry>` | `GrpcRegistryClient` |
| `secret_store_factory` | `Arc<dyn SecretStore>` | `NatsSecretStore` |
| `state_factory` | `Arc<dyn DagStore>` | `GrpcStateClient` |

## Workers

14 worker processes spawned by the supervisor:

| Worker | Type | Service |
|--------|------|---------|
| Registry | gRPC server | `PersistentRegistry` |
| State | gRPC server | `SqliteDagStore` |
| Harness | gRPC server | `CoreHarness` |
| Secret | gRPC server | `NatsSecretStore` |
| Catalog | gRPC server | `CompositeCatalog` |
| Agent Container | tick loop | `ContainerRuntime` |
| Inference Ollama | tick loop | `OllamaWorker` |
| Inference OpenRouter | tick loop | `OpenRouterWorker` |
| Object Storage SQLite | tick loop | `KvWorker` |
| Object Storage Memory | tick loop | `KvWorker` |
| Vector Storage SQLite | tick loop | `SqliteVecWorker` |
| Vector Storage Memory | tick loop | `SqliteVecWorker` |
| DAG Git | NATS consumer | `GitDagWorker` |
| Session Viewer | HTTP server | `SessionServer` |

---

## Core Trait Inventory

| Trait | Purpose | Implementations |
|-------|---------|-----------------|
| `Registry` | System state, agent/job tracking | `PersistentRegistry`, `GrpcRegistryClient`, *`InMemoryRegistry`* |
| `RegistryRepository` | Registry persistence | `SqliteRegistryRepository` |
| `MessageQueue` | Typed message passing | `NatsQueue`, `RecordingQueue`, *`InMemoryQueue`* |
| `Runtime` | Agent execution | `ContainerRuntime` (Podman) |
| `DagStore` | Merkle DAG persistence | `SqliteDagStore`, `GrpcStateClient`, *`InMemoryDagStore`* |
| `DagWorker` | DAG message processing | `GitDagWorker` |
| `Harness` | Agent interaction API | `CoreHarness`, `GrpcHarnessClient` |
| `ModelCatalog` | Model name resolution | `OllamaCatalog`, `OpenRouterCatalog` |
| `CatalogService` | Multi-backend catalog | `CompositeCatalog` |
| `SecretStore` | Named secret storage | `NatsSecretStore`, `GrpcSecretClient`, *`InMemorySecretStore`* |

*Italicized* = test double only.

**Design principle**: Every trait has one production implementation per backend and one in-memory test double. Production always flows through NATS + gRPC (ADR 062).

---

## Relationships

### Composition

```
Agent
 +-- id: ResourceId
 +-- runtime: RuntimeType
 +-- executable: String (OCI image ref)
 +-- image_digest: Option<ImageDigest>
 +-- public_key: Option<Vec<u8>>
 +-- Requirements
 |    +-- models: Map<String, ResourceId>
 |    +-- services: Map<String, ServiceConfig>
 |    +-- mounts: Map<String, MountConfig>
 +-- prompts: Option<Prompts>
 +-- object_storage: Option<ObjectStorageType>
 +-- vector_storage: Option<VectorStorageType>

Model
 +-- id: ResourceId
 +-- model_type: ModelType (Inference | Embedding)
 +-- provider: Provider (Ollama | OpenRouter)
 +-- model_path: ResourceId
 +-- digest: String

DagNode
 +-- hash: String (SHA-256 Merkle chain)
 +-- parent_hash: String
 +-- message_type: MessageType
 +-- from / to: String
 +-- session_id / submission_id: String
 +-- payload / diagnostics / stderr: Vec<u8>
 +-- state: Option<String>
```

### Dependencies

```
vlinder daemon
 +-- Supervisor (process manager)
      +-- NatsQueue (shared message bus)
      +-- Registry worker -> PersistentRegistry -> SqliteRegistryRepository
      |    +-- RegistryServiceServer (gRPC)
      +-- State worker -> SqliteDagStore
      |    +-- StateServiceServer (gRPC)
      +-- Harness worker -> CoreHarness
      |    +-- HarnessServiceServer (gRPC)
      +-- Agent worker -> ContainerRuntime (Podman)
      +-- Inference workers -> OllamaWorker / OpenRouterWorker
      +-- Storage workers -> KvWorker / SqliteVecWorker
      +-- DAG git worker -> GitDagWorker -> git repo
      +-- Session viewer -> SessionServer (HTTP)

vlinder (CLI client)
 +-- GrpcHarnessClient -> Harness worker (gRPC)
 +-- GrpcRegistryClient -> Registry worker (gRPC)
```
