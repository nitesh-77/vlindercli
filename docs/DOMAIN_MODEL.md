# Domain Model

This document describes the core types and abstractions in VlinderCLI. It separates the **domain** (abstract protocol) from **implementations** (concrete infrastructure).

For architectural decisions, see `docs/adr/`. For the vision, see `VISION.md`.

---

## Architecture Overview

VlinderCLI uses a **queue-based message-passing architecture** (ADR 018). Everything is a service that consumes messages and produces responses:

```
┌─────────────────────────────────────────────────────────────┐
│                     Message Queue (NATS)                    │
│                                                             │
│   ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌────────────┐        │
│   │inference│ │embedding│ │ storage │ │  agents    │        │
│   │ worker  │ │ worker  │ │ workers │ │(containers)│        │
│   └─────────┘ └─────────┘ └─────────┘ └────────────┘        │
│   ┌─────────┐ ┌─────────┐                                   │
│   │  state  │ │   DAG   │                                   │
│   │ worker  │ │ worker  │                                   │
│   └─────────┘ └─────────┘                                   │
│        ▲           ▲           ▲           ▲                │
│        └───────────┴───────────┴───────────┘                │
│                    all just workers                         │
└─────────────────────────────────────────────────────────────┘
```

**Architecture**: `vlinder daemon` runs a **Supervisor** that spawns worker processes. Workers connect to **NATS** for messaging and **gRPC** for registry/state. There is no in-process mode — all execution flows through the queue (ADR 062).

In-memory implementations (`InMemoryQueue`, `InMemoryRegistry`, etc.) exist only as **unit test doubles**.

---

# Part 1: Domain (Abstract Protocol)

The domain defines **what** the system does: types, traits, and contracts. Infrastructure-agnostic.

Location: [`src/domain/`](../src/domain/mod.rs)

---

## Identity Types

These types provide identity and ordering across the system.

### ResourceId

URI-based resource identity. Key for looking up any resource in the registry: storage, models, runtimes, agents.

- **Type**: [`src/domain/resource_id.rs`](../src/domain/resource_id.rs)
- Scheme indicates resource type: `sqlite://`, `ollama://`, `openrouter://`, `memory://`, `http://`
- Supports parsing into `scheme()`, `authority()`, and `path()` components

### MessageId, SubmissionId, SessionId

Tracking identifiers for message flow (ADR 044, 054):

- `MessageId` — unique per message (UUID)
- `SubmissionId` — groups messages for one user request. Content-addressed SHA-256 hash of (payload, session_id, parent_submission) per ADR 081
- `SessionId` — groups submissions into a conversation. Format: `ses-{uuid}`

### Sequence / SequenceCounter

Ordering for service requests within a submission. Starts at 1, increments per request. Thread-safe counter allocates sequential values.

### ImageRef, ImageDigest

OCI container identity types with parse-time validation:

- `ImageRef` — validated OCI image reference (e.g., `localhost/echo-agent:latest`). Must contain `/`.
- `ImageDigest` — validated content-addressed digest (e.g., `sha256:a80c4f17...`). Only `sha256:` supported.

- **Types**: [`src/domain/image_ref.rs`](../src/domain/image_ref.rs), [`src/domain/image_digest.rs`](../src/domain/image_digest.rs)

---

## Configuration Types

These types describe **what** to use.

### Agent

A unit of behavior with declared requirements: identity, executable (OCI image), models, services, mounts, prompts. Mounts are Podman volume mounts mapping host paths to guest paths.

- **Type**: [`src/domain/agent.rs`](../src/domain/agent.rs)
- Key fields: `name`, `runtime: RuntimeType`, `executable: String` (OCI image ref), `image_digest: Option<ImageDigest>`, `requirements: Requirements`, `object_storage`, `vector_storage`

### Requirements

What an agent needs to run: models (name → ResourceId mapping) and services (by name).

- **Type**: [`src/domain/agent.rs`](../src/domain/agent.rs) (defined alongside Agent)

### Mount

Podman volume mount: maps host path to guest path with read/write mode.

- **Type**: [`src/domain/agent.rs`](../src/domain/agent.rs) (defined alongside Agent)

### Prompts

Optional LLM prompt overrides for agent behavior customization.

- **Type**: [`src/domain/agent.rs`](../src/domain/agent.rs) (defined alongside Agent)

### Model

An inference or embedding capability: name, type, engine, model path (ResourceId), content digest.

- **Type**: [`src/domain/model.rs`](../src/domain/model.rs)
- `EngineType`: `Ollama` | `OpenRouter` | `InMemory` (test-only)
- `ModelType`: `Inference` | `Embedding`

### Fleet

A composition boundary for agents: name, entry-point agent, project directory, agents map (name → path).

- **Type**: [`src/domain/fleet.rs`](../src/domain/fleet.rs)

### Storage

Storage configuration types and traits for object (key-value) and vector (embedding) storage.

- **Type**: [`src/domain/storage.rs`](../src/domain/storage.rs)
- `ObjectStorageType`: `Sqlite` | `InMemory` (test-only)
- `VectorStorageType`: `SqliteVec` | `InMemory` (test-only)
- `ObjectStorage` trait: `put_file()`, `get_file()`, `delete_file()`, `list_files()`
- `VectorStorage` trait: `store_embedding()`, `search_by_vector()`, `delete_embedding()`

---

## Protocol Types

The five typed messages that flow through the queue (ADR 044). Every message carries `protocol_version`, `submission`, and `session` for traceability.

### Message Flow

```
Harness ──InvokeMessage──▶ Runtime ──RequestMessage──▶ Service
                                   ◀──ResponseMessage──
        ◀──CompleteMessage──

Agent ──DelegateMessage──▶ Agent (via runtime)
```

### InvokeMessage

Harness → Runtime. Starts a submission by invoking an agent. Carries `agent_id`, `harness: HarnessType`, `runtime: RuntimeType`, `state: Option<String>`, and `InvokeDiagnostics`.

### RequestMessage

Runtime → Service. Agent requests a service operation (kv, vec, infer, embed). Carries `service`, `backend`, `operation`, `sequence`, and `RequestDiagnostics`.

### ResponseMessage

Service → Runtime. Service replies with result. Echoes all request dimensions plus `correlation_id` for tracing. Carries `ServiceDiagnostics`.

### CompleteMessage

Runtime → Harness. Submission finished. Carries final `state` hash and `ContainerDiagnostics`.

### DelegateMessage

Agent → Agent (via runtime). One agent invoking another. Carries `caller_agent`, `target_agent`, `reply_subject`, and `DelegateDiagnostics`.

- **Types**: [`src/domain/message.rs`](../src/domain/message.rs)

### ExpectsReply Trait

Type-level enforcement of request-response pairing:
- `InvokeMessage` expects `CompleteMessage`
- `RequestMessage` expects `ResponseMessage`
- Terminal messages (`CompleteMessage`, `ResponseMessage`) do NOT implement `ExpectsReply` — attempting to reply is a compile error.

### ObservableMessage

Unified enum wrapping all five message types for polymorphic handling (e.g., receiving from a queue when the type isn't known at compile time).

### HarnessType

Entry point variants: `Cli`, `Web`, `Api`, `Whatsapp`. Routes completion messages back to the correct harness. Only `Cli` is currently implemented; the other variants are reserved for future use.

### MessageQueue Trait

Typed send/receive methods for all five message types, plus routing helpers and delegation support.

- **Trait**: [`src/domain/message_queue.rs`](../src/domain/message_queue.rs)

### SdkContract Trait

The 11 operations agents can request (ADR 074, 075):

| Category | Operations |
|----------|------------|
| Object storage | `kv_get`, `kv_put`, `kv_list`, `kv_delete` |
| Vector storage | `vector_store`, `vector_search`, `vector_delete` |
| Inference | `infer` |
| Embedding | `embed` |
| Delegation | `delegate`, `wait` |

- **Trait**: [`src/domain/sdk.rs`](../src/domain/sdk.rs)

### AgentAction / AgentEvent

JSON wire format for state-machine agents (ADR 074):
- `AgentAction` — agent → platform (POST /handle response body). Tagged with `"action"`.
- `AgentEvent` — platform → agent (POST /handle request body). Tagged with `"type"`.
- Each carries an opaque `state: Value` field that the platform round-trips without interpretation.

### QueueBridge

Queue-backed implementation of `SdkContract`. Routes typed SDK calls through the `MessageQueue`. Manages state cursor for versioned KV operations (ADR 055).

- **Type**: [`src/domain/queue_bridge.rs`](../src/domain/queue_bridge.rs)

---

## Time Travel Types

Content-addressed data structures for runtime tracing and state versioning.

### DagNode / DagStore Trait

Merkle DAG for runtime tracing (ADR 067). One `DagNode` per NATS message — no pairing, each message is independently meaningful.

- `hash`: `SHA-256(payload || parent_hash || message_type || diagnostics)` — Merkle chain
- `parent_hash`: previous node in the session (empty for root)
- `message_type`: `Invoke` | `Request` | `Response` | `Complete` | `Delegate`
- `from` / `to`: sender and receiver
- `session_id`, `submission_id`, `payload`, `diagnostics`, `stderr`, `state`, `protocol_version`

`DagStore` trait: `insert_node()`, `get_node()`, `get_session_nodes()`, `get_children()`, `latest_state()`, `latest_node_hash()`.

- **Types**: [`src/domain/dag.rs`](../src/domain/dag.rs)

### StateCommit / State Hashing

Content-addressed versioned state mirroring git's object model (ADR 055):

| Concept | Git equivalent | Hash function |
|---------|---------------|---------------|
| Value | Blob | `SHA-256(content)` |
| Snapshot | Tree | `SHA-256(sorted JSON of path→value_hash)` |
| State commit | Commit | `SHA-256(snapshot_hash + ":" + parent_hash)` |

Root state is empty string `""` — the parent of the first commit.

- **Types**: [`src/domain/state.rs`](../src/domain/state.rs)

### Session / HistoryEntry

Conversation state for multi-turn interactions (ADR 054). `Session` groups multiple submissions, builds enriched payloads with conversation history, and tracks open questions.

- **Types**: [`src/domain/session.rs`](../src/domain/session.rs)

### Route / Stop

Protocol trace for a session. `Route` is the full chain of messages; each `Stop` is one message with hash, type, sender, receiver, payload, and timestamp. Built from `DagNode`s.

- **Types**: [`src/domain/route.rs`](../src/domain/route.rs)

---

## Diagnostics

Each message type carries diagnostics specific to its emitter (ADR 071). The type system encodes what each platform component guarantees — no shared struct with optional fields.

| Message | Diagnostics type | Emitter |
|---------|------------------|---------|
| Invoke | `InvokeDiagnostics` | Harness |
| Request | `RequestDiagnostics` | QueueBridge |
| Response | `ServiceDiagnostics` + `ServiceMetrics` | Service workers |
| Complete | `ContainerDiagnostics` + `ContainerRuntimeInfo` | Container runtime |
| Delegate | `DelegateDiagnostics` | Container runtime |

`ServiceMetrics` is a tagged enum: `Inference { tokens_input, tokens_output, model }`, `Embedding { dimensions, model }`, `Storage { operation, bytes_transferred }`.

`ContainerRuntimeInfo` captures Podman metadata: engine version, image ref, image digest, container ID.

- **Types**: [`src/domain/diagnostics.rs`](../src/domain/diagnostics.rs)

---

## Control Plane Types

These types manage system state and coordinate execution.

### Registry Trait

Source of truth for all system state. Stores agents, models, jobs, and tracks available capabilities (runtimes, storage types, engine types).

- **Trait**: [`src/domain/registry.rs`](../src/domain/registry.rs)
- Agent operations: `register_agent()`, `get_agent()`, `get_agents()`, `select_runtime()`
- Model operations: `register_model()`, `get_model()`, `delete_model()`
- Job operations: `create_job()`, `get_job()`, `update_job_status()`, `pending_jobs()`
- Capability registration/queries for runtimes, storage types, engine types

### RegistryRepository Trait

Persistence layer for Registry state. Handles save/load of agents and models to durable storage (SQLite, etc).

- **Trait**: [`src/domain/registry_repository.rs`](../src/domain/registry_repository.rs)

### Job / JobId / JobStatus

A submitted job and its current state.

- `JobId`: format `<registry_id>/jobs/<uuid>`
- `JobStatus`: `Pending` → `Running` → `Completed(String)` | `Failed(String)`

### Harness Trait

API surface for agent deployment and job management. Currently only `CliHarness` implements this trait. `HarnessType` has additional variants (Web, API, WhatsApp) reserved for future use.

- `deploy()` — register agent from TOML manifest
- `invoke()` — submit a job
- `poll()` — check job status

- **Trait**: [`src/domain/harness.rs`](../src/domain/harness.rs)

### Runtime Trait / RuntimeType

Agent execution protocol. The runtime discovers agents from Registry, polls their input queues, and executes agent code on message arrival.

- `RuntimeType`: `Container` (OCI containers via Podman)
- Methods: `id()`, `runtime_type()`, `tick()`, `shutdown()`

- **Trait**: [`src/domain/runtime.rs`](../src/domain/runtime.rs)

### ModelCatalog Trait

Resolves model names to Model configurations from backend catalogs.

- Methods: `resolve()`, `list()`, `available()`
- **Trait**: [`src/domain/catalog.rs`](../src/domain/catalog.rs)

---

## Service Workers

Service workers are **domain entities** that define protocol handlers (ADR 030). They subscribe to queue subjects, process typed requests, and send typed responses. Inference and embedding workers validate that agents declared the model before invoking.

Location: [`src/domain/workers/`](../src/domain/workers/mod.rs)

| Worker | Service Queues | Trait Used |
|--------|---------------|------------|
| `ObjectServiceWorker` | `kv` — get, put, list, delete | `ObjectStorage` |
| `VectorServiceWorker` | `vec` — store, search, delete | `VectorStorage` |
| `InferenceServiceWorker` | `infer` — run | `InferenceEngine` |
| `EmbeddingServiceWorker` | `embed` — run | `EmbeddingEngine` |
| `GitDagWorker` | DAG node utilities | `DagStore` |

---

## Loader Trait

Abstracts how configuration is loaded from URIs. Free functions dispatch by URI scheme.

- **Trait**: [`src/loader.rs`](../src/loader.rs)

```rust
pub trait Loader {
    fn load_agent(&self, uri: &str) -> Result<Agent, LoadError>;
    fn load_fleet(&self, uri: &str) -> Result<Fleet, LoadError>;
    fn load_model(&self, uri: &str) -> Result<Model, LoadError>;
}
```

---

## Path Types

Enforce absolute paths at compile time. Relative paths in manifests resolve against the manifest's directory at load time.

- **Types**: [`src/domain/path.rs`](../src/domain/path.rs)

---

## Core Trait Inventory

| Trait | Purpose | Implementations |
|-------|---------|-----------------|
| `Registry` | System state, agent/job tracking | `PersistentRegistry`, `GrpcRegistryClient`, *`InMemoryRegistry`* (test) |
| `RegistryRepository` | Registry persistence | `SqliteRegistryRepository` |
| `MessageQueue` | Typed message passing | `NatsQueue`, `RecordingQueue`, *`InMemoryQueue`* (test) |
| `Runtime` | Agent execution | `ContainerRuntime` (Podman) |
| `InferenceEngine` | Text generation | `OllamaInferenceEngine`, `OpenRouterInferenceEngine`, *`InMemoryInference`* (test) |
| `EmbeddingEngine` | Vector embeddings | `OllamaEmbeddingEngine`, *`InMemoryEmbedding`* (test) |
| `ObjectStorage` | Key-value file storage | `SqliteObjectStorage`, *`InMemoryObjectStorage`* (test) |
| `VectorStorage` | Embedding search | `SqliteVectorStorage`, *`InMemoryVectorStorage`* (test) |
| `DagStore` | Merkle DAG persistence | `SqliteDagStore`, `GrpcStateClient` |
| `SdkContract` | Agent SDK operations | `QueueBridge` |
| `ModelCatalog` | Model name resolution | `OllamaCatalog`, `OpenRouterCatalog` |
| `Harness` | Agent interaction API | `CliHarness` |
| `Loader` | Load configs from URIs | `FileLoader` |

**Design principle**: Every trait has one production implementation per backend and one in-memory test double. Production always flows through NATS + gRPC (ADR 062). Agent code interacts only with `SdkContract` and never sees infrastructure.

---

## Relationships

### Composition ("has a")

```
Agent
 ├── id: ResourceId
 ├── runtime: RuntimeType
 ├── executable: String (OCI image ref)
 ├── image_digest: Option<ImageDigest>
 ├── Requirements
 │    ├── models: Map<String, ResourceId>
 │    └── services: Vec<String>
 ├── mounts: Vec<Mount>
 ├── prompts: Option<Prompts>
 ├── object_storage: Option<ResourceId>
 └── vector_storage: Option<ResourceId>

Model
 ├── id: ResourceId
 ├── model_type: ModelType (Inference | Embedding)
 ├── engine: EngineType (Ollama | OpenRouter | InMemory)
 ├── model_path: ResourceId
 └── digest: String

Fleet
 ├── name: String
 ├── entry: String
 ├── project_dir: PathBuf
 └── agents: Map<String, PathBuf>

DagNode
 ├── hash: String (SHA-256 Merkle chain)
 ├── parent_hash: String
 ├── message_type: MessageType
 ├── from / to: String
 ├── session_id / submission_id: String
 ├── payload: Vec<u8>
 ├── diagnostics: Vec<u8>
 ├── stderr: Vec<u8>
 ├── state: Option<String>
 └── protocol_version: String
```

### Implementation ("implements")

Production implementations listed first; *italicized* = test-only double.

```
InferenceEngine              ObjectStorage              MessageQueue
 ├── OllamaInferenceEngine    ├── SqliteObjectStorage    ├── NatsQueue
 ├── OpenRouterInferenceEngine├── *InMemoryObjectStorage* ├── RecordingQueue
 └── *InMemoryInference*                                  └── *InMemoryQueue*

EmbeddingEngine              VectorStorage              DagStore
 ├── OllamaEmbeddingEngine    ├── SqliteVectorStorage    ├── SqliteDagStore
 └── *InMemoryEmbedding*      └── *InMemoryVectorStorage* └── GrpcStateClient

Registry                     Runtime                    SdkContract
 ├── PersistentRegistry        └── ContainerRuntime       └── QueueBridge
 ├── GrpcRegistryClient
 └── *InMemoryRegistry*

ModelCatalog                 Loader                     Harness
 ├── OllamaCatalog            └── FileLoader             └── CliHarness
 └── OpenRouterCatalog
```

### Dependencies ("uses")

```
vlinder daemon
 └── Supervisor (process manager)
      ├── NatsQueue (shared message bus)
      ├── Registry worker → PersistentRegistry → SqliteRegistryRepository
      │    └── RegistryServiceServer (gRPC)
      ├── State worker → SqliteDagStore
      │    └── StateServiceServer (gRPC)
      ├── Agent worker → ContainerRuntime (Podman)
      │    └── QueueBridge → routes SDK calls through NatsQueue
      ├── Inference worker → OllamaInferenceEngine / OpenRouterInferenceEngine
      ├── Embedding worker → OllamaEmbeddingEngine
      ├── Object storage worker → SqliteObjectStorage
      ├── Vector storage worker → SqliteVectorStorage
      └── DAG git worker → writes conversation commits

vlinder agent run (CLI client)
 └── CliHarness
      ├── GrpcRegistryClient → Registry worker (gRPC)
      └── NatsQueue → submits InvokeMessage, polls for CompleteMessage
```

### Loading Flow

```
URI ("file://./agent.toml")
 └── Loader (dispatches by scheme)
      └── FileLoader
           └── AgentManifest (TOML deserialization)
                └── Agent (resolved domain type)
```

---

# Part 2: Implementations (Concrete Infrastructure)

These modules implement the domain concepts with actual infrastructure.

---

## Inference Implementations

Location: [`src/inference/`](../src/inference/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `OllamaInferenceEngine` | `InferenceEngine` | HTTP client for Ollama's inference API |
| `OpenRouterInferenceEngine` | `InferenceEngine` | OpenAI-compatible HTTP client for OpenRouter cloud LLMs |
| *`InMemoryInference`* | `InferenceEngine` | Returns canned response — **test double only** |

Factory: `open_inference_engine(model)` selects engine by `EngineType`.

---

## Embedding Implementations

Location: [`src/embedding/`](../src/embedding/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `OllamaEmbeddingEngine` | `EmbeddingEngine` | HTTP client for Ollama's embedding API |
| *`InMemoryEmbedding`* | `EmbeddingEngine` | Returns canned vector — **test double only** |

Factory: `open_embedding_engine(model)` selects engine by `EngineType`.

---

## Storage Implementations

Location: [`src/storage/`](../src/storage/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `SqliteObjectStorage` | `ObjectStorage` | SQLite-backed virtual filesystem |
| *`InMemoryObjectStorage`* | `ObjectStorage` | HashMap-based — **test double only** |
| `SqliteVectorStorage` | `VectorStorage` | sqlite-vec extension for similarity search |
| *`InMemoryVectorStorage`* | `VectorStorage` | Brute-force euclidean distance — **test double only** |
| `StateStore` | — | Content-addressed SQLite store for versioned state (ADR 055) |
| `SqliteDagStore` | `DagStore` | SQLite-backed Merkle DAG persistence |
| `SqliteRegistryRepository` | `RegistryRepository` | SQLite persistence for registry state |

---

## Queue Implementations

Location: [`src/queue/`](../src/queue/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `NatsQueue` | `MessageQueue` | NATS JetStream with sync facade over async tokio |
| `RecordingQueue` | `MessageQueue` | Decorator — records DAG nodes before forwarding |
| *`InMemoryQueue`* | `MessageQueue` | VecDeque per subject — **test double only** |

Factories: `from_config()` and `recording_from_config()`.

---

## Runtime

The `Runtime` trait defines agent execution protocol. Only container runtime exists.

- **Trait**: [`src/domain/runtime.rs`](../src/domain/runtime.rs)

### ContainerRuntime

Executes OCI container agents via Podman. Tick-loop orchestrator: polls invoke/delegate queues, dispatches to containers via HTTP POST `/handle`, manages the `AgentAction`/`AgentEvent` state-machine loop, and uses `QueueBridge` for SDK calls.

- **Implementation**: [`src/runtime/container/`](../src/runtime/container/mod.rs)
- **Podman interface**: `Podman` trait with `PodmanCli` production implementation
- **Container lifecycle**: `ContainerPool` manages start/stop/reuse

---

## Registry Implementations

Location: [`src/registry/`](../src/registry/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `PersistentRegistry` | `Registry` | Write-through to SQLite via `SqliteRegistryRepository` |
| *`InMemoryRegistry`* | `Registry` | RwLock-guarded in-memory state — **test double only** |

### gRPC (distributed access)

Location: [`src/registry_service/`](../src/registry_service/mod.rs)

| Implementation | Role | Description |
|----------------|------|-------------|
| `GrpcRegistryClient` | Client | Implements `Registry` trait via gRPC calls |
| `RegistryServiceServer` | Server | Wraps a `Registry` impl, serves gRPC requests |

---

## State Service

Location: [`src/state_service/`](../src/state_service/mod.rs)

| Implementation | Role | Description |
|----------------|------|-------------|
| `GrpcStateClient` | Client | Implements `DagStore` trait via gRPC calls |
| `StateServiceServer` | Server | Wraps a `DagStore` impl, serves gRPC requests |

---

## Model Catalog Implementations

Location: [`src/catalog/`](../src/catalog/)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `OllamaCatalog` | `ModelCatalog` | Resolves models from local Ollama instance |
| `OpenRouterCatalog` | `ModelCatalog` | Resolves models from OpenRouter API |

---

## Loader Implementations

Location: [`src/loader.rs`](../src/loader.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `FileLoader` | `Loader` | Loads from filesystem (`file://` scheme) |

### TOML Manifests

`FileLoader` uses these types to deserialize TOML files:

| Type | File Format | Location |
|------|-------------|----------|
| `AgentManifest` | `agent.toml` | [`src/domain/agent_manifest.rs`](../src/domain/agent_manifest.rs) |
| `ModelManifest` | `<model>.toml` | [`src/domain/model_manifest.rs`](../src/domain/model_manifest.rs) |
| `FleetManifest` | `fleet.toml` | [`src/domain/fleet_manifest.rs`](../src/domain/fleet_manifest.rs) |

---

## Supervisor

Location: [`src/supervisor.rs`](../src/supervisor.rs)

Process manager for `vlinder daemon`. Spawns worker processes, waits for readiness (registry first, then state service), and manages shutdown. This is the only deployment mode (ADR 062).

---

# Summary

```
┌─────────────────────────────────────────────────────────────┐
│                      DOMAIN (abstract)                      │
│                                                             │
│  Identity: ResourceId, MessageId, SubmissionId, SessionId,  │
│            ImageRef, ImageDigest, Sequence                  │
│  Config:   Agent, Model, Fleet, Storage                     │
│  Protocol: InvokeMessage, RequestMessage, ResponseMessage,  │
│            CompleteMessage, DelegateMessage                 │
│  SDK:      SdkContract (11 ops), AgentAction, AgentEvent    │
│  Time:     DagNode, DagStore, StateCommit, Session, Route   │
│  Control:  Registry, Harness, Runtime, Job, ModelCatalog    │
│  Workers:  Object, Vector, Inference, Embedding, GitDag     │
│  Diag:     Invoke/Request/Service/Container/Delegate Diag   │
└─────────────────────────────────────────────────────────────┘
                            │
                            │ implements
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                  IMPLEMENTATIONS (concrete)                 │
│                                                             │
│  Inference: OllamaInferenceEngine, OpenRouterInferenceEngine│
│  Embedding: OllamaEmbeddingEngine                           │
│  Storage:   SqliteObjectStorage, SqliteVectorStorage,       │
│             StateStore, SqliteDagStore                      │
│  Queue:     NatsQueue, RecordingQueue                       │
│  Runtime:   ContainerRuntime (Podman)                       │
│  Registry:  PersistentRegistry, GrpcRegistryClient          │
│  State:     GrpcStateClient, StateServiceServer             │
│  Catalog:   OllamaCatalog, OpenRouterCatalog                │
│  Loader:    FileLoader                                      │
│  (InMemory* impls exist as test doubles only)               │
└─────────────────────────────────────────────────────────────┘

Deployment (vlinder daemon → Supervisor):

┌─────────────────────────────────────────────────────────────┐
│  Supervisor spawns worker processes                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐     │
│  │ Registry │  │  State   │  │  Agent   │  │ Service  │     │
│  │  (gRPC)  │  │  (gRPC)  │  │ workers  │  │ workers  │     │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘     │
│        ▲              ▲            ▲              ▲         │
│        └──────────────┴────────────┴──────────────┘         │
│                         NATS                                │
└─────────────────────────────────────────────────────────────┘
```
