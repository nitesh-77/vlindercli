# ADR 030: Domain Module Structure

## Status

Accepted

## Context

ADR 018 established queue-based message passing. The question remained: where do the queue handlers (service workers) belong? Initially they lived in `src/runtime/services/`, implying they're runtime implementation details.

But service workers define **what messages are valid** and **how the protocol works**. They depend only on domain traits (`ObjectStorage`, `VectorStorage`, `InferenceEngine`, `EmbeddingEngine`), not concrete implementations. The protocol is fixed; backends are swappable.

## Decision

### Service Workers are Domain Entities

Service workers moved from `src/runtime/services/` to `src/domain/workers/`.

**Rationale**: They define the protocol handlers—the fixed vocabulary of operations (`kv-get`, `kv-put`, `vector-store`, `vector-search`, `infer`, `embed`). Concrete backends (SQLite, S3, llama.cpp) are infrastructure; workers are protocol.

```
domain/workers/
├── object.rs      # kv-get, kv-put, kv-delete, kv-list
├── vector.rs      # vector-store, vector-search, vector-delete
├── inference.rs   # infer
└── embedding.rs   # embed
```

### Provider Aggregates Workers

`Provider` is a domain struct that aggregates service workers and routes messages to registered backends.

```rust
pub struct Provider {
    object: ObjectServiceWorker,
    vector: VectorServiceWorker,
    inference: InferenceServiceWorker,
    embedding: EmbeddingServiceWorker,
}
```

Provider supports **heterogeneous deployments**: different agents can use different backends within the same Provider instance. Registration maps namespaces/models to backend implementations.

### Runtime is a Trait

`Runtime` defines the agent execution protocol:

```rust
pub trait Runtime {
    fn register(&mut self, agent: Agent);
    fn tick(&mut self) -> bool;
}
```

`WasmRuntime` implements this trait. Future runtimes (Lambda, K8s) will implement the same interface.

### Interior Mutability for Registration

Workers use `RwLock<HashMap<...>>` internally, allowing registration via `&self`:

```rust
pub fn register(&self, namespace: &str, storage: Arc<dyn ObjectStorage>) {
    self.stores.write().unwrap().insert(namespace.to_string(), storage);
}
```

This enables Provider to be shared across components without requiring `&mut self` for setup.

## Module Structure

```
src/domain/
├── mod.rs              # re-exports all domain types
├── agent.rs            # Agent value type
├── agent_manifest.rs   # TOML deserialization
├── model.rs            # Model value type
├── model_manifest.rs   # TOML deserialization
├── fleet.rs            # Fleet value type
├── fleet_manifest.rs   # TOML deserialization
├── storage.rs          # ObjectStorage, VectorStorage traits + configs
├── inference.rs        # InferenceEngine trait + config
├── embedding.rs        # EmbeddingEngine trait + config
├── resource_id.rs      # URI-based registry key
├── path.rs             # AbsolutePath, AbsoluteUri
├── provider.rs         # Service worker aggregation
├── runtime.rs          # Runtime trait
├── harness.rs          # Harness trait + CliHarness
└── workers/
    ├── mod.rs
    ├── object.rs       # ObjectServiceWorker
    ├── vector.rs       # VectorServiceWorker
    ├── inference.rs    # InferenceServiceWorker
    └── embedding.rs    # EmbeddingServiceWorker
```

## Consequences

- **Protocol is in domain**: Workers define the message vocabulary, not backends
- **Backends are infrastructure**: SQLite, S3, llama.cpp live in implementation modules
- **Heterogeneous deployment**: One Provider serves multiple agents with different backends
- **Runtime is abstract**: Can swap WasmRuntime for other implementations
- **Registration is non-mutating**: Shared ownership via `&self` methods
