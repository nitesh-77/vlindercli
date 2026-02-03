# ADR 036: Storage Lifecycle

## Status

Accepted

## Context

Agents need persistent storage for files (object storage) and embeddings (vector storage). The system must manage the lifecycle of these storage instances: when they're created, who creates them, where they live, and how they're cleaned up.

Storage is fundamentally **stateful and per-agent**. Agent A's files must be isolated from Agent B's files. This is different from inference/embedding engines, which are stateless and can be shared.

## Decision

### Storage is Per-Agent

Each agent gets its own isolated storage instances:

```
Agent "pensieve"  → pensieve's object storage + pensieve's vector storage
Agent "summarizer" → summarizer's object storage + summarizer's vector storage
```

No sharing. No cross-agent access.

### Manifest Declares Storage Location

The agent manifest specifies where storage lives:

```toml
object_storage = "sqlite://./data/objects.db"
vector_storage = "sqlite://./data/vectors.db"
```

Relative paths resolve against the agent's directory. This gives agent authors control over their data layout.

### Workers Own Lifecycle Logic

Workers manage their own storage instances:

```
ObjectServiceWorker
  → get_or_open(agent_id, registry) → storage instance
  → looks up agent.object_storage URI from Registry
  → opens storage if not cached
  → caches instance for reuse
```

Workers ask Registry for agent info. Registry is the single source of truth for all state including storage URIs. Provider is just a bag of workers - no lifecycle logic.

### Workers Invoke Services

When an agent calls `put_file()` or `store_embedding()`:

1. Worker receives the request
2. Worker asks the appropriate service for a storage instance
3. Service returns existing instance, or opens a new one
4. Worker performs the operation

Workers are backend-agnostic. They use the `ObjectStorage` or `VectorStorage` trait without knowing what's behind it.

### Lazy Opening

Storage opens on first use, not at agent registration. An agent that never uses storage never gets storage allocated.

### Backend Implementations Own Create-or-Connect Semantics

Each backend handles its own initialization:

**SQLite:**
```rust
impl SqliteObjectStorage {
    pub fn open(path: &Path) -> Result<Self> {
        // Creates file if missing, opens if exists
        let conn = Connection::open(path)?;
        conn.execute(CREATE_TABLES_IF_NOT_EXISTS)?;
        Ok(Self { conn })
    }
}
```

**In-Memory:**
```rust
impl InMemoryObjectStorage {
    pub fn new() -> Self {
        // Always fresh, ephemeral
        Self { data: HashMap::new() }
    }
}
```

**S3:**
```rust
impl S3ObjectStorage {
    pub fn connect(uri: &str) -> Result<Self> {
        // Connects to existing bucket, fails if missing
        let client = S3Client::new(uri)?;
        client.head_bucket()?;
        Ok(Self { client })
    }
}
```

The service layer stays simple—it just dispatches based on URI scheme:

```rust
fn open(&mut self, uri: &str) -> Result<Arc<dyn ObjectStorage>> {
    match scheme(uri) {
        "sqlite" => Ok(Arc::new(SqliteObjectStorage::open(path)?)),
        "memory" => Ok(Arc::new(InMemoryObjectStorage::new())),
        "s3"     => Ok(Arc::new(S3ObjectStorage::connect(uri)?)),
        _        => Err("unknown storage scheme"),
    }
}
```

### Local vs Cloud Backends

| Type | Backends | Behavior |
|------|----------|----------|
| **Local** | SQLite, in-memory | Open-or-create. Works out of the box. |
| **Cloud** | S3, Pinecone, etc. | BYOS (Bring Your Own Storage). User provisions the resource; we connect. |

We do not provision cloud infrastructure. Cloud users bring pre-existing buckets, indexes, or tables.

### Component Responsibilities

| Component | Does | Does NOT |
|-----------|------|----------|
| **Daemon** | Supervises processes (spawn, monitor, restart) | Create storage, hold state, invoke business logic |
| **Registry** | Stores ALL state (agents, jobs, URIs) | Create resources |
| **Provider** | Bag of workers | Hold state, lifecycle logic |
| **Workers** | Ask Registry for info, lazy-open storage, perform operations | — |
| **Backends** | Implement traits, handle create-or-connect | — |

## Consequences

- **Isolation guaranteed**: Per-agent storage, no leakage between agents
- **Backend-agnostic workers**: Workers use traits, don't know SQLite from S3
- **Local backends work immediately**: SQLite + in-memory need no external setup
- **Cloud is BYOS**: We connect, we don't provision
- **Lazy allocation**: Agents that don't use storage don't get storage allocated
- **Clear ownership**: Workers lazy-open, Registry holds state, Daemon supervises

---

## Implementation

### Current State

| Aspect | Current | Target |
|--------|---------|--------|
| Storage per-agent | ✓ Keyed by agent_id in workers | Same |
| Manifest declares URI | ✓ `object_storage`, `vector_storage` fields on Agent | Same |
| Lifecycle logic location | ✗ In `Daemon::register_agent_infrastructure()` | In Workers |
| Creation timing | ✗ Eager (at agent registration) | Lazy (on first use) |
| Dispatch by scheme | ✗ Hardcoded to in-memory | Workers dispatch based on URI scheme |
| Backend create-or-connect | ✓ `SqliteObjectStorage::open()` exists | Same |

### Changes Required

**1. Give workers Registry access**

Workers need to look up agent info to get storage URIs:

```rust
impl ObjectServiceWorker {
    pub fn new(queue: Arc<dyn MessageQueue>, registry: Arc<RwLock<Registry>>) -> Self {
        Self { queue, registry, stores: RwLock::new(HashMap::new()) }
    }

    fn get_or_open(&self, agent_id: &str) -> Result<Arc<dyn ObjectStorage>, String> {
        // Check cache first
        if let Some(storage) = self.stores.read().unwrap().get(agent_id) {
            return Ok(storage.clone());
        }

        // Look up URI from Registry
        let registry = self.registry.read().unwrap();
        let agent = registry.get_agent(agent_id)
            .ok_or_else(|| format!("unknown agent: {}", agent_id))?;
        let uri = agent.object_storage.as_ref()
            .ok_or_else(|| format!("agent has no object_storage declared"))?;

        // Open and cache
        let storage = open_from_uri(uri)?;
        self.stores.write().unwrap().insert(agent_id.to_string(), storage.clone());
        Ok(storage)
    }
}
```

**2. Remove eager creation from Daemon**

In `register_agent_infrastructure()`, delete:

```rust
// DELETE THIS:
let storage = in_memory_storage();
let object = open_object_storage(&storage).expect("...");
let vector = open_vector_storage(&storage).expect("...");
self.provider.register_object(name, object);
self.provider.register_vector(name, vector);
```

Daemon just registers the agent (with its manifest URIs) in Registry. Workers lazy-open on first use.

**3. Remove register methods from Provider**

Provider no longer needs `register_object`/`register_vector` - workers manage their own caches.

### Files to Modify

| File | Change |
|------|--------|
| `src/domain/workers/object.rs` | Add Registry access, `get_or_open()` method |
| `src/domain/workers/vector.rs` | Add Registry access, `get_or_open()` method |
| `src/domain/provider.rs` | Pass Registry to workers, remove `register_object`/`register_vector` |
| `src/domain/daemon.rs` | Remove eager storage creation, pass Registry to Provider |

---

## Open Questions

- **Destruction**: When does storage close? (Deferred - happens when agent is deregistered)

## Resolved

- **Worker → URI**: Workers ask Registry. Registry is the single source of truth for all state.
- **Missing URI**: Fail. If agent calls storage without declaring it in manifest, error: "agent has no object_storage declared". Explicit is better than implicit.
