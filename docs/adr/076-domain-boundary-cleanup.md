# ADR 076: Domain Boundary Cleanup

## Status

Draft

## Context

Vlinder is an infrastructure orchestration platform. The domain *is* infrastructure — message queues, container runtimes, storage backends, content-addressed state. A `MessageQueue` is to Vlinder what an `Order` is to an e-commerce system.

But the codebase doesn't reflect this. Domain types are scattered across implementation modules:

- The five protocol message types (`InvokeMessage`, `RequestMessage`, etc.) live in `crate::queue` alongside NATS and InMemory implementations
- `DagStore`, `DagNode`, `StateCommit`, and content-addressing functions live in `crate::storage` alongside SQLite implementations
- `HttpBridge` (which does no HTTP — it routes `AgentBridge` calls through `MessageQueue`) lives in `crate::bridge` even though every dependency is a domain entity
- `InMemoryRegistry`, `PersistentRegistry`, and `CliHarness` are implementations living inside `src/domain/`

The pattern is consistent: every module that needs cleanup has domain abstractions mixed with concrete implementations. Meanwhile, `runtime/`, `catalog/`, `inference/`, `embedding/`, and `registry_service/` are already clean — they contain only implementations.

The forcing function: ADR 075 (State Machine Agents) and timeline repair both need `AgentAction`, `AgentEvent`, message types, and `DagNode` from outside the runtime. These types are protocol, not implementation. Having them in implementation modules forces awkward cross-module imports and `pub(crate)` visibility where `pub` is correct.

## Decision

**Move all domain types into `src/domain/`. Move all implementations out.**

After cleanup, `src/domain/` contains: all traits, all protocol types, all identity types, all content-addressing functions, and the bridge composition. Everything else is an implementation that belongs in its own module.

### 1. `crate::queue` → extract domain types

Move to `src/domain/`:

- **`MessageQueue` trait** and `QueueError` — the platform's communication protocol
- **Five message types**: `InvokeMessage`, `RequestMessage`, `ResponseMessage`, `CompleteMessage`, `DelegateMessage`
- **Identity types**: `MessageId`, `SubmissionId`, `SessionId`, `Sequence`, `SequenceCounter`, `HarnessType`
- **Protocol types**: `ExpectsReply` trait, `ObservableMessage` enum
- **Diagnostics**: `InvokeDiagnostics`, `RequestDiagnostics`, `ServiceDiagnostics`, `ServiceMetrics`, `ContainerDiagnostics`, `ContainerRuntimeInfo`, `DelegateDiagnostics`
- **`agent_routing_key()`** — domain routing logic

Keep in `crate::queue` (implementations):

- `InMemoryQueue` — single-process implementation
- `NatsQueue` — distributed implementation
- `from_config()` — factory/wiring function

### 2. `src/bridge/` → move to domain

`HttpBridge` implements `AgentBridge` by routing through `MessageQueue` using the five message types. Every dependency is a domain entity. It's domain composition, not infrastructure.

The name changes too — it does no HTTP. It becomes `QueueBridge` (or just `Bridge`): the standard composition of `AgentBridge` over `MessageQueue`.

### 3. `crate::storage` → extract domain types

Move to `src/domain/`:

- **`DagStore` trait** — persistence abstraction for the Merkle DAG
- **`DagNode`** — one message in the protocol trace
- **`MessageType` enum** — protocol classification (Invoke, Request, Response, Complete, Delegate)
- **`hash_dag_node()`** — content-addressing (Merkle chain)
- **`StateCommit`** — versioned state concept (ADR 055)
- **`hash_value()`, `hash_snapshot()`, `hash_state_commit()`** — content-addressing functions

Keep in `crate::storage` (implementations):

- `SqliteDagStore`, `StateStore` — SQLite implementations
- `SqliteObjectStorage`, `InMemoryObjectStorage` — `ObjectStorage` implementations
- `SqliteVectorStorage`, `InMemoryVectorStorage` — `VectorStorage` implementations
- `SqliteRegistryRepository` — `RegistryRepository` implementation
- `dispatch.rs` — factory/wiring functions

### 4. `src/domain/` → move implementations out

These are implementations that currently live inside the domain module:

| Implementation | Trait it implements | Move to |
|---|---|---|
| `InMemoryRegistry` | `Registry` | `crate::queue` or a new `crate::registry` module |
| `PersistentRegistry` | `Registry` (wraps `SqliteRegistryRepository`) | `crate::storage` or `crate::registry` |
| `CliHarness` | `Harness` | `crate::commands` (it's CLI-specific) |

### What stays where it is

These modules are already clean — pure implementations of domain traits:

- `src/runtime/` — Podman container orchestration, state machine dispatch
- `src/registry_service/` — gRPC transport for `Registry`
- `src/catalog/` — Ollama and OpenRouter API clients for `ModelCatalog`
- `src/embedding/` — Ollama embedding client for `EmbeddingEngine`
- `src/inference/` — Ollama/OpenRouter inference clients for `InferenceEngine`
- `src/services/` — worker logic between queue and domain traits
- `src/commands/` — CLI layer

## Consequences

- `src/domain/` becomes the single source of truth for the platform's abstract protocol — all traits, all message types, all identity types, all content-addressing
- Implementation modules (`queue`, `storage`, `runtime`, `catalog`, etc.) contain only concrete code that implements domain traits
- Cross-module imports become clean: everything depends on `crate::domain`, never on `crate::queue::message` or `crate::storage::dag_store` for protocol types
- The `HttpBridge` → `QueueBridge` rename makes the architecture legible — the bridge routes through the queue, not through HTTP
- Existing tests move with their types — no behavioral changes
- This is a large mechanical refactor with no behavioral changes — every type keeps its exact shape, only its module path changes
