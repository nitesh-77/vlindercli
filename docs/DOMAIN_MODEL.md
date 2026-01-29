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

## Capability Traits

These traits define the **runtime protocol**. They are the contracts that implementations must fulfill.

| Trait | Purpose | Definition |
|-------|---------|------------|
| `InferenceEngine` | Text generation from prompts | [`src/domain/inference.rs`](../src/domain/inference.rs) |
| `EmbeddingEngine` | Text to vector embeddings | [`src/domain/embedding.rs`](../src/domain/embedding.rs) |
| `ObjectStorage` | Key-value file storage | [`src/domain/storage.rs`](../src/domain/storage.rs) |
| `VectorStorage` | Embedding storage with similarity search | [`src/domain/storage.rs`](../src/domain/storage.rs) |
| `MessageQueue` | Message passing abstraction | [`src/queue/traits.rs`](../src/queue/traits.rs) |
| `Harness` | User-facing interface to runtime | [`src/domain/harness.rs`](../src/domain/harness.rs) |
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

EmbeddingEngine          VectorStorage              Harness
 ├── LlamaEmbeddingEngine ├── SqliteVecStorage       └── CliHarness
 └── InMemoryEmbedding    └── InMemoryVectorStorage

Loader
 └── FileLoader
```

### Dependencies ("uses")

```
CliHarness
 └── uses WasmRuntime
      └── uses MessageQueue
      └── uses ServiceHandlers
           ├── InferenceServiceHandler → InferenceEngine
           ├── EmbeddingServiceHandler → EmbeddingEngine
           ├── ObjectServiceHandler → ObjectStorage
           └── VectorServiceHandler → VectorStorage
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

Location: [`src/runtime/`](../src/runtime/mod.rs)

**Note**: No abstract `Runtime` trait exists yet. `WasmRuntime` is currently the only implementation.

### WasmRuntime

Executes WASM agents via Extism. Provides host functions: `send`, `receive`, `get_prompts`.

- **Implementation**: [`src/runtime/wasm.rs`](../src/runtime/wasm.rs)

### Service Handlers

Native handlers that listen on well-known queues:

| Handler | Queues | Location |
|---------|--------|----------|
| `InferenceServiceHandler` | `infer` | [`src/runtime/services/inference.rs`](../src/runtime/services/inference.rs) |
| `EmbeddingServiceHandler` | `embed` | [`src/runtime/services/embedding.rs`](../src/runtime/services/embedding.rs) |
| `ObjectServiceHandler` | `kv-*` | [`src/runtime/services/object.rs`](../src/runtime/services/object.rs) |
| `VectorServiceHandler` | `vector-*` | [`src/runtime/services/vector.rs`](../src/runtime/services/vector.rs) |

---

## Harness

Location: [`src/domain/harness.rs`](../src/domain/harness.rs)

| Implementation | Description |
|----------------|-------------|
| `CliHarness` | Embeds `WasmRuntime` in-process for CLI usage |

**Future**: `DaemonHarness` (connects to vlinderd via socket)

---

# Summary

```
┌─────────────────────────────────────────────────────────────┐
│                      DOMAIN (abstract)                      │
│                                                             │
│  Types: Agent, Model, Fleet, Storage                        │
│  Traits: InferenceEngine, EmbeddingEngine, ObjectStorage,   │
│          VectorStorage, MessageQueue, Harness, Loader       │
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
│  Queue:     InMemoryQueue, Message                          │
│  Loader:    FileLoader                                      │
│  Runtime:   WasmRuntime, ServiceHandlers                    │
│  Harness:   CliHarness                                      │
└─────────────────────────────────────────────────────────────┘
```
