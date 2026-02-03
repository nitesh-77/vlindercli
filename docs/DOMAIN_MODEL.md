# Domain Model

This document describes the core types and abstractions in VlinderCLI. It separates the **domain** (abstract protocol) from **implementations** (concrete infrastructure).

For architectural decisions, see `docs/adr/`. For the vision, see `VISION.md`.

---

## Architecture Overview

VlinderCLI uses a **queue-based message-passing architecture** (see ADR 018). Everything is a service that consumes messages and produces responses:

```
┌─────────────────────────────────────────────────────────────┐
│                        Message Queue                        │
│                                                             │
│   ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐           │
│   │inference│ │embedding│ │ storage │ │ agents  │           │
│   │ service │ │ service │ │ service │ │         │           │
│   └─────────┘ └─────────┘ └─────────┘ └─────────┘           │
│        ▲           ▲           ▲           ▲                │
│        └───────────┴───────────┴───────────┘                │
│                    all just workers                         │
└─────────────────────────────────────────────────────────────┘
```

**Key insight**: The same protocol works for local (in-process queue) and distributed (Redis, SQS) deployment. Scaling means changing infrastructure, not rewriting agents.

---

# Part 1: Domain (Abstract Protocol)

The domain defines **what** the system does—types, traits, and contracts. Infrastructure-agnostic.

Location: [`src/domain/`](../src/domain/mod.rs)

---

## Configuration Types

These types describe **what** to use.

### Agent

A unit of behavior with declared requirements: identity, code, models, services, mounts, prompts.

- **Type**: [`src/domain/agent.rs`](../src/domain/agent.rs)

### Requirements

What an agent needs to run: models (by name → manifest URI) and services (by name).

- **Type**: [`src/domain/agent.rs`](../src/domain/agent.rs) (defined alongside Agent)

### Mount

WASI filesystem mount: maps host path to guest path with read/write mode.

- **Type**: [`src/domain/agent.rs`](../src/domain/agent.rs) (defined alongside Agent)

### Prompts

Optional LLM prompt overrides for agent behavior customization.

- **Type**: [`src/domain/agent.rs`](../src/domain/agent.rs) (defined alongside Agent)

### Model

An inference or embedding capability: name, type, engine, weights URI.

- **Type**: [`src/domain/model.rs`](../src/domain/model.rs)

### Fleet

A deployment configuration for agents: name, agent references, storage config.

- **Type**: [`src/domain/fleet.rs`](../src/domain/fleet.rs)

### Storage

Storage configuration: backend type (SQLite, InMemory), connection details.

- **Type**: [`src/domain/storage.rs`](../src/domain/storage.rs)

---

## Control Plane Types

These types manage system state and coordinate execution.

### Daemon

The control plane that owns all system components. Like k8s, it coordinates Registry (etcd), Harness (API Server), Runtime (Kubelet), and Provider (services).

- **Type**: [`src/domain/daemon.rs`](../src/domain/daemon.rs)

### Registry

Source of truth for all system state. Stores jobs, agents, models, and tracks available capabilities (runtimes, storage types, engine types). Has an `id` (ResourceId) representing its API endpoint.

- **Type**: [`src/domain/registry.rs`](../src/domain/registry.rs)

### Job

A submitted job and its current state: id (JobId), agent_id (ResourceId), input, status.

- **Type**: [`src/domain/registry.rs`](../src/domain/registry.rs)

### JobId

Unique identifier for a job. Format: `<registry_id>/jobs/<uuid>` (e.g., `http://127.0.0.1:9000/jobs/abc-123`).

- **Type**: [`src/domain/registry.rs`](../src/domain/registry.rs)

### JobStatus

Job lifecycle states: Pending → Running → Completed/Failed.

- **Type**: [`src/domain/registry.rs`](../src/domain/registry.rs)

### Harness (Daemon-owned)

API surface owned by Daemon. Handles agent deployment (from filesystem or TOML), job submission, and status tracking via Registry.

- **Type**: [`src/domain/harness.rs`](../src/domain/harness.rs)

---

## Loader Trait

The `Loader` trait abstracts how configuration is loaded from URIs. Different schemes dispatch to different implementations.

- **Trait**: [`src/loader.rs`](../src/loader.rs)

```rust
pub trait Loader {
    fn load_agent(&self, uri: &str) -> Result<Agent, LoadError>;
    fn load_fleet(&self, uri: &str) -> Result<Fleet, LoadError>;
    fn load_model(&self, uri: &str) -> Result<Model, LoadError>;
}
```

Free functions (`load_agent()`, `load_fleet()`, `load_model()`) dispatch by URI scheme to the appropriate loader implementation.

---

## Core Trait Inventory

These traits define the system's extension points. Each trait has one or more implementations that can be swapped for different deployment scenarios.

| Trait | Purpose | In-Process Impl | Future/Network Impl |
|-------|---------|-----------------|---------------------|
| `Registry` | System state, agent/job tracking | `InMemoryRegistry` | `RegistryClient` (HTTP) |
| `MessageQueue` | Message passing | `InMemoryQueue` | Redis, SQS |
| `Runtime` | Agent execution | `WasmRuntime` | Lambda, K8s |
| `InferenceEngine` | Text generation | `LlamaEngine` | Ollama, OpenAI |
| `EmbeddingEngine` | Vector embeddings | `LlamaEmbeddingEngine` | Ollama, OpenAI |
| `ObjectStorage` | File storage | `SqliteObjectStorage`, `InMemoryObjectStorage` | S3 |
| `VectorStorage` | Embedding search | `SqliteVecStorage`, `InMemoryVectorStorage` | Pinecone, Qdrant |
| `Loader` | Load configs from URIs | `FileLoader` | S3, Consul |

**Design principle**: Local development uses in-process implementations. Production swaps to network implementations. Agent code doesn't change.

---

## Capability Traits

These traits define the **runtime protocol**. They are the contracts that implementations must fulfill.

| Trait | Purpose | Definition |
|-------|---------|------------|
| `Registry` | System state and coordination | [`src/domain/registry.rs`](../src/domain/registry.rs) |
| `InferenceEngine` | Text generation from prompts | [`src/domain/inference.rs`](../src/domain/inference.rs) |
| `EmbeddingEngine` | Text to vector embeddings | [`src/domain/embedding.rs`](../src/domain/embedding.rs) |
| `ObjectStorage` | Key-value file storage | [`src/domain/storage.rs`](../src/domain/storage.rs) |
| `VectorStorage` | Embedding storage with similarity search | [`src/domain/storage.rs`](../src/domain/storage.rs) |
| `MessageQueue` | Message passing abstraction | [`src/queue/traits.rs`](../src/queue/traits.rs) |
| `Runtime` | Agent execution protocol | [`src/domain/runtime.rs`](../src/domain/runtime.rs) |
| `Loader` | Load agents/fleets/models from URIs | [`src/loader.rs`](../src/loader.rs) |

---

## Path Types

Enforce absolute paths at compile time. Relative paths in manifests resolve against the manifest's directory at load time.

- **Types**: [`src/domain/path.rs`](../src/domain/path.rs)

---

## Relationships

### Composition ("has a")

```
Agent
 ├── Requirements
 │    └── models: Map<String, AbsoluteUri>
 │    └── services: Vec<String>
 ├── mounts: Vec<Mount>
 ├── prompts: Option<Prompts>
 └── code: AbsoluteUri

Model
 ├── model_type: ModelType
 └── engine: ModelEngine

Fleet
 ├── agents: Vec<AgentRef>
 └── storage: Storage
      └── backend: StorageBackend
           └── kind: StorageKind
```

### Implementation ("implements")

```
InferenceEngine          ObjectStorage              MessageQueue
 ├── LlamaEngine          ├── SqliteObjectStorage    └── InMemoryQueue
 └── InMemoryInference    └── InMemoryObjectStorage

EmbeddingEngine          VectorStorage              Runtime
 ├── LlamaEmbeddingEngine ├── SqliteVecStorage       └── WasmRuntime
 └── InMemoryEmbedding    └── InMemoryVectorStorage

Loader
 └── FileLoader
```

### Dependencies ("uses")

```
Daemon (control plane)
 ├── owns Registry
 │    ├── stores Job, Agent, Model
 │    └── tracks available capabilities
 ├── owns Harness
 │    └── API surface for deploy/invoke/poll
 ├── owns WasmRuntime (implements Runtime trait)
 │    ├── discovers agents from Registry
 │    └── uses MessageQueue
 └── owns Provider (aggregates workers)
      ├── ObjectServiceWorker → Registry → ObjectStorage (lazy)
      ├── VectorServiceWorker → Registry → VectorStorage (lazy)
      ├── InferenceServiceWorker → Registry → InferenceEngine (lazy)
      └── EmbeddingServiceWorker → Registry → EmbeddingEngine (lazy)
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

## Loader Implementations

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `FileLoader` | `Loader` | Loads from filesystem (`file://` scheme) |

- **Location**: [`src/loader.rs`](../src/loader.rs)

| URI Scheme | Loader | Status |
|------------|--------|--------|
| `file://` | `FileLoader` | Implemented |
| `s3://`, `consul://`, etc. | — | Future |

### TOML Manifests

`FileLoader` uses these types to deserialize TOML files:

| Type | File Format | Location |
|------|-------------|----------|
| `AgentManifest` | `agent.toml` | [`src/domain/agent_manifest.rs`](../src/domain/agent_manifest.rs) |
| `ModelManifest` | `<model>.toml` | [`src/domain/model_manifest.rs`](../src/domain/model_manifest.rs) |
| `FleetManifest` | `<fleet>-fleet.toml` | [`src/domain/fleet_manifest.rs`](../src/domain/fleet_manifest.rs) |

**Future**: JSON, YAML file formats.

---

## Inference Implementations

Location: [`src/inference/`](../src/inference/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `LlamaEngine` | `InferenceEngine` | llama.cpp via llama-cpp-2 crate |
| `InMemoryInference` | `InferenceEngine` | Returns canned response (testing) |

**Note**: Inference and embedding share a single `OnceLock<LlamaBackend>` to avoid llama.cpp reinitialization errors.

---

## Embedding Implementations

Location: [`src/embedding/`](../src/embedding/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `LlamaEmbeddingEngine` | `EmbeddingEngine` | llama.cpp embedding models |
| `InMemoryEmbedding` | `EmbeddingEngine` | Returns canned vector (testing) |

---

## Storage Implementations

Location: [`src/storage/`](../src/storage/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `SqliteObjectStorage` | `ObjectStorage` | SQLite-backed file storage |
| `SqliteVecStorage` | `VectorStorage` | sqlite-vec for similarity search |
| `InMemoryObjectStorage` | `ObjectStorage` | HashMap-based (testing) |
| `InMemoryVectorStorage` | `VectorStorage` | Vec-based brute force (testing) |

---

## Queue Implementations

Location: [`src/queue/`](../src/queue/mod.rs)

| Implementation | Trait | Description |
|----------------|-------|-------------|
| `InMemoryQueue` | `MessageQueue` | Channel-based in-process queue |

### Message

The `Message` struct is defined in [`src/queue/message.rs`](../src/queue/message.rs). It includes ID, payload, reply-to queue, and correlation ID for request-response pairing.

**Note**: `Message` is a concrete struct in the queue module, not a domain abstraction.

**Future**: Redis, SQS for distributed deployment.

---

## Runtime

The `Runtime` trait defines agent execution protocol. See ADR 030.

- **Trait**: [`src/domain/runtime.rs`](../src/domain/runtime.rs)

### WasmRuntime

Executes WASM agents via Extism. Discovers agents from Registry, polls their input queues, executes WASM on message arrival. Provides host functions `send` and `get_prompts` to guests.

- **Implementation**: [`src/runtime/wasm.rs`](../src/runtime/wasm.rs)

---

## Service Workers (Domain)

Service workers are **domain entities** that define the protocol handlers (see ADR 030). They have Registry access and lazy-load resources on first use (see ADR 036). Inference and embedding workers validate agents declared the model before invoking.

Location: [`src/domain/workers/`](../src/domain/workers/mod.rs)

| Worker | Queues | Trait Used |
|--------|--------|------------|
| `ObjectServiceWorker` | `kv-get`, `kv-put`, `kv-delete`, `kv-list` | `ObjectStorage` |
| `VectorServiceWorker` | `vector-store`, `vector-search`, `vector-delete` | `VectorStorage` |
| `InferenceServiceWorker` | `infer` | `InferenceEngine` |
| `EmbeddingServiceWorker` | `embed` | `EmbeddingEngine` |

---

## Provider (Domain)

`Provider` aggregates service workers and routes messages to registered backends.

- **Location**: [`src/domain/provider.rs`](../src/domain/provider.rs)

Supports heterogeneous deployments: different agents can use different backends within the same Provider instance.

---

## Harness

Location: [`src/domain/harness.rs`](../src/domain/harness.rs)

`Harness` is a struct that provides the API surface for agent deployment and job management. Owned by `Daemon`, it deploys agents, submits jobs to Registry, queues messages to Runtime, and reconciles completed jobs from the reply queue.

The CLI (`vlinder agent run`) uses Daemon which owns Harness internally.

---

# Summary

```
┌─────────────────────────────────────────────────────────────┐
│                      DOMAIN (abstract)                      │
│                                                             │
│  Config: Agent, Model, Fleet, Storage, ResourceId           │
│  Control: Daemon, Registry, Harness, Job, JobId, JobStatus  │
│  Traits: InferenceEngine, EmbeddingEngine, ObjectStorage,   │
│          VectorStorage, MessageQueue, Runtime               │
│  Workers: ObjectServiceWorker, VectorServiceWorker,         │
│           InferenceServiceWorker, EmbeddingServiceWorker    │
│  Provider: aggregates workers, routes to backends           │
└─────────────────────────────────────────────────────────────┘
                            │
                            │ implements
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                  IMPLEMENTATIONS (concrete)                 │
│                                                             │
│  Inference: LlamaEngine, InMemoryInference                  │
│  Embedding: LlamaEmbeddingEngine, InMemoryEmbedding         │
│  Storage:   SqliteObjectStorage, SqliteVecStorage,          │
│             InMemoryObjectStorage, InMemoryVectorStorage    │
│  Queue:     InMemoryQueue                                   │
│  Runtime:   WasmRuntime                                     │
│  Loader:    FileLoader                                      │
└─────────────────────────────────────────────────────────────┘

Control Plane (k8s-like architecture):

┌─────────────────────────────────────────────────────────────┐
│                         Daemon                              │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐    │
│  │ Registry │  │ Harness  │  │ Runtime  │  │ Provider │    │
│  │ (state)  │  │  (API)   │  │ (exec)   │  │(services)│    │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘    │
└─────────────────────────────────────────────────────────────┘
```
