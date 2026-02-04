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

## Open Questions

- Stream naming convention? (`vlinder.jobs.<agent_id>`, `vlinder.events.*`)
- Consumer group strategy for scaling workers?
- How to handle NATS connection failures gracefully?
