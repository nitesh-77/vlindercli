# ADR 041: NATS Queue Integration

## Status

Proposed

## Context

The current queue implementation (`InMemoryQueue`) is single-process only. This limits Vlinder to:

1. **Single daemon** — no distributed agents
2. **No persistence** — messages lost on crash
3. **No pub/sub** — only point-to-point messaging

To unlock distributed agents, durable job queues, and event streaming, we need a real messaging system.

### Why NATS?

- **Simple** — single binary, minimal config
- **Fast** — designed for high throughput, low latency
- **JetStream** — built-in persistence and exactly-once delivery
- **Request/reply native** — matches our `Message` pattern (reply_to, correlation_id)
- **Leaf nodes** — future path to edge/hybrid deployments

### The Async Problem

NATS clients (`async-nats`) are async. Our `MessageQueue` trait is sync:

```rust
pub trait MessageQueue {
    fn send(&self, queue: &str, msg: Message) -> Result<(), QueueError>;
    fn receive(&self, queue: &str) -> Result<Message, QueueError>;
}
```

Options considered:
1. **Make trait async** — clean but breaking, infects entire codebase
2. **Block internally** — pragmatic, sync API hides async engine
3. **Separate traits** — duplication, consumers must handle both

## Decision

### 1. External NATS Server

NATS runs as external infrastructure, not embedded. An `install.sh` script handles setup:

```
install.sh
├── Installs Ollama (inference)
└── Installs NATS (messaging)
```

**Default endpoints:**
- Ollama: `http://localhost:11434`
- NATS: `nats://localhost:4222`

This avoids embedding complexity and lets users manage services independently.

### 2. Sync Facade, Async Engine

Developer-facing API is sync. Async machinery is hidden inside:

```
Developer writes:                 System executes:
────────────────                  ────────────────

queue.send("jobs", msg);    →     ┌──────────────────┐
                                  │  Tokio Runtime   │
let msg = queue.recv("jobs");     │  ┌────────────┐  │
                                  │  │async-nats  │  │
// Sync, simple, obvious          │  │ client     │  │
                                  │  └────────────┘  │
                                  └──────────────────┘
```

### 3. NatsQueue Implementation

```rust
pub struct NatsQueue {
    inner: Arc<NatsQueueInner>,
}

struct NatsQueueInner {
    runtime: tokio::runtime::Runtime,
    client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
}

impl NatsQueue {
    pub fn connect(url: &str) -> Result<Self, QueueError> {
        let runtime = tokio::runtime::Runtime::new()?;
        let (client, jetstream) = runtime.block_on(async {
            let client = async_nats::connect(url).await?;
            let jetstream = async_nats::jetstream::new(client.clone());
            Ok((client, jetstream))
        })?;

        Ok(Self {
            inner: Arc::new(NatsQueueInner { runtime, client, jetstream })
        })
    }
}

impl MessageQueue for NatsQueue {
    fn send(&self, queue: &str, msg: Message) -> Result<(), QueueError> {
        self.inner.runtime.block_on(self.send_async(queue, msg))
    }

    fn receive(&self, queue: &str) -> Result<Message, QueueError> {
        self.inner.runtime.block_on(self.receive_async(queue))
    }
}
```

**Source**: `src/queue/nats.rs`

### 4. Async Escape Hatch

Power users can access the async client directly:

```rust
impl NatsQueue {
    /// Escape hatch for async contexts
    pub fn async_client(&self) -> &async_nats::Client {
        &self.inner.client
    }

    pub fn jetstream(&self) -> &async_nats::jetstream::Context {
        &self.inner.jetstream
    }
}
```

### 5. JetStream for Durability

Use JetStream streams for durable queues:

| Queue Type | NATS Primitive | Use Case |
|------------|----------------|----------|
| Ephemeral | Core NATS subject | Fast, fire-and-forget |
| Durable | JetStream stream | Job queues, must not lose |
| Request/reply | NATS request | Sync call pattern |

### 6. Configuration

```toml
# .vlinder/config.toml
[queue]
backend = "nats"  # or "memory" for local dev
nats_url = "nats://localhost:4222"
```

Or via environment:
```bash
VLINDER_QUEUE_BACKEND=nats
VLINDER_NATS_URL=nats://localhost:4222
```

### 7. Migration Path

```
Phase 1 (MVP):     Sync facade with block_on
Phase 2 (Later):   AsyncMessageQueue trait alongside sync
Phase 3 (Future):  Full async codebase, deprecate sync
```

The sync facade buys time. When the codebase goes async (HTTP server, WASM async), the internal implementation stays the same — only the trait boundary changes.

## Consequences

**Positive:**
- Distributed agents — multiple daemons share work
- Durable queues — jobs survive crashes via JetStream
- Sync simplicity — developers write straightforward code
- Future-proof — async internals ready for full async migration
- Escape hatch — power users get raw async access

**Negative:**
- External dependency — NATS must be running
- Thread blocking — sync facade blocks OS threads (acceptable for MVP)
- Two runtimes — main thread sync, NatsQueue owns tokio (isolated)

## Subject and Stream Design

### Subject Naming

Use agent/model **names** (not full URIs) for subjects. URIs contain invalid characters (`://`, `/`) and are payload data, not routing keys.

```
vlinder.agent.{name}       Agent invocations (e.g., vlinder.agent.pensieve)
vlinder.svc.infer          Inference requests
vlinder.svc.embed          Embedding requests
vlinder.svc.kv             Object storage operations
vlinder.svc.vec            Vector storage operations
_INBOX.{id}                Replies (Core NATS ephemeral inbox)
```

### Stream Configuration

Start with one stream capturing everything:

```rust
stream::Config {
    name: "VLINDER",
    subjects: vec!["vlinder.>"],
    retention: RetentionPolicy::WorkQueue,
    storage: StorageType::File,
    ..Default::default()
}
```

**Why one stream:**
- Wildcard `vlinder.>` captures all subjects
- No per-agent/per-service setup needed
- Refine into separate streams later if needed (observability, different retention)

### Reply Pattern

Use NATS native request/reply with ephemeral inboxes:

```rust
// Request with auto-generated reply inbox
let response = client.request("vlinder.svc.infer", payload).await?;
```

NATS creates `_INBOX.{unique_id}` automatically. No JetStream needed for replies—they're ephemeral and short-lived.

### Queue Name Mapping

| Current (InMemoryQueue) | NATS Subject |
|-------------------------|--------------|
| `"infer"` | `vlinder.svc.infer` |
| `"embed"` | `vlinder.svc.embed` |
| `"kv-get"`, `"kv-put"`, etc. | `vlinder.svc.kv` |
| `"vector-store"`, etc. | `vlinder.svc.vec` |
| `"{agent_id}"` | `vlinder.agent.{name}` |
| `"{reply_to}"` | `_INBOX.{id}` (auto) |

### Why This Is Safe

All decisions are reversible:

| Change | Cost |
|--------|------|
| Rename subjects | Update string, redeploy |
| Split stream | Create new stream, update subjects |
| Add durability | Add JetStream to Core NATS subject |
| Change retention | Update stream config |

No one-way doors.

## Open Questions

- Consumer group strategy for scaling workers?
- How to handle NATS connection failures gracefully?
- Metrics/observability for queue depth?
