# ADR 035: Registry Capability Tracking

## Status

Accepted

## Context

ADR 031 established vlinderd/Registry as the central authority. ADR 034 defined runtime dispatch based on agent URI patterns.

The system has multiple capability types that agents may require:
- **Runtimes** — execute agent code (Wasm, Lambda, Container)
- **Object storage** — file persistence (SQLite, S3, in-memory)
- **Vector storage** — embedding persistence (SQLite+vec, Pinecone, in-memory)
- **Inference engines** — text generation (Llama, Ollama, Bedrock)
- **Embedding engines** — vector generation (Nomic, OpenAI, in-memory)

Each capability type has multiple implementations. At startup, we need to know which implementations are actually available before we can dispatch requests to them.

## Decision

**Registry tracks available implementations for each capability type.**

Each capability type has a corresponding `*Type` enum:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeType { Wasm }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectStorageType { Sqlite, InMemory }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VectorStorageType { SqliteVec, InMemory }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InferenceEngineType { Llama, InMemory }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmbeddingEngineType { Nomic, InMemory }
```

Registry stores availability in `HashSet<*Type>` fields:

```rust
pub struct Registry {
    available_runtimes: HashSet<RuntimeType>,
    available_object_storage: HashSet<ObjectStorageType>,
    available_vector_storage: HashSet<VectorStorageType>,
    available_inference_engines: HashSet<InferenceEngineType>,
    available_embedding_engines: HashSet<EmbeddingEngineType>,
}
```

**Registration happens at Daemon startup:**

```rust
impl Daemon {
    pub fn new() -> Self {
        let mut registry = Registry::new();

        // Register what's actually available
        registry.register_runtime(RuntimeType::Wasm);
        registry.register_object_storage(ObjectStorageType::Sqlite);
        registry.register_object_storage(ObjectStorageType::InMemory);
        registry.register_vector_storage(VectorStorageType::SqliteVec);
        registry.register_vector_storage(VectorStorageType::InMemory);
        registry.register_inference_engine(InferenceEngineType::Llama);
        registry.register_inference_engine(InferenceEngineType::InMemory);
        registry.register_embedding_engine(EmbeddingEngineType::Nomic);
        registry.register_embedding_engine(EmbeddingEngineType::InMemory);
        // ...
    }
}
```

**Query methods check availability:**

```rust
impl Registry {
    pub fn has_object_storage(&self, t: ObjectStorageType) -> bool;
    pub fn has_vector_storage(&self, t: VectorStorageType) -> bool;
    pub fn has_inference_engine(&self, t: InferenceEngineType) -> bool;
    pub fn has_embedding_engine(&self, t: EmbeddingEngineType) -> bool;
}
```

## Consequences

- **Two-layer dispatch**: Registration (what exists) is separate from selection (what to use)
- **Fail-fast**: Can reject agent registration if required capability is unavailable
- **Extensible**: Adding new implementations means adding enum variant + registration call
- **Compile-time safety**: `*Type` enums ensure only known implementations are referenced
- **Selection logic deferred**: Only `RuntimeType` has `select_runtime()` so far; storage/engine selection can follow the same pattern when needed

## Relationship to Other ADRs

- **ADR 031**: Defines Registry as source of truth — this ADR specifies what it tracks
- **ADR 034**: Uses `available_runtimes` for dispatch — same pattern applies to other capabilities
