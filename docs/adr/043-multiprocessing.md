# ADR 043: Multi-Processing Architecture

## Status

Proposed

## Context

The current architecture runs everything in a single process with tick-based execution. This limits:

1. **Scalability** — can't add more inference workers without running more daemons
2. **Isolation** — a slow inference blocks everything else
3. **Resource utilization** — can't dedicate machines to specific workloads

With NATS integrated (ADR 041), we can distribute work across processes.

### Key Insight: Stateless Workers

Inference and embedding workers are **stateless HTTP proxies** to Ollama (or similar services). They don't load models into memory — Ollama does. This means:

- No model pinning needed in worker config
- Workers are interchangeable, scale freely
- Complexity lives in engine selection, not worker topology

The Llama.cpp engine exists only for integration testing. It validates abstractions but is not production-ready.

## Decision

### 1. Deployment Modes

```toml
[deployment]
mode = "local"        # Default: single process, tick-based
# mode = "distributed"  # Multi-process via NATS
```

| Mode | Queue | Workers | Use Case |
|------|-------|---------|----------|
| `local` | memory or nats | In-process | Development, testing |
| `distributed` | nats (required) | Separate processes | Production, scaling |

### 2. Configuration

```toml
# ~/.vlinder/config.toml

[logging]
level = "info"
llama_level = "error"

[ollama]
endpoint = "http://localhost:11434"

[queue]
backend = "nats"
nats_url = "nats://localhost:4222"

[deployment]
mode = "distributed"
harness = true        # Run API on this machine (default: true)

# Agent runtimes (by backend type)
[workers.agent]
wasm = 4              # WASM executors
docker = 2            # Docker containers
# firecracker = 0     # Not configured on this machine

# Inference engines (by backend type)
[workers.infer]
ollama = 2            # Ollama proxy (stateless)

# Embedding engines (by backend type)
[workers.embed]
ollama = 1            # Ollama proxy (stateless)

# Object storage (by backend type)
[workers.storage.object]
sqlite = 1            # Local file, single writer
# s3 = 4              # Remote proxy, scale freely

# Vector storage (by backend type)
[workers.storage.vector]
sqlite-vec = 1        # Local file
# pinecone = 4        # Remote proxy, scale freely
```

**Key insight:** Backends fall into two categories:

| Type | Examples | Characteristics |
|------|----------|-----------------|
| Local execution | WASM, Docker, Firecracker, SQLite | Runs on worker machine, resource constrained |
| Remote proxy | Ollama, S3, Pinecone, OpenAI | Stateless HTTP, scale freely |

Local backends have deployment constraints. Remote backends are just proxies — scale by adding workers.

NATS subjects include backend type for routing:
```
vlinder.agent.wasm.{name}
vlinder.agent.docker.{name}
vlinder.svc.infer.ollama.{model}
vlinder.svc.kv.sqlite.{instance}
vlinder.svc.kv.s3.{instance}
vlinder.svc.vec.sqlite-vec.{instance}
vlinder.svc.vec.pinecone.{instance}
```

Workers subscribe to backends they can handle. A machine with Docker subscribes to `vlinder.agent.docker.>`. A machine without it simply doesn't.

### 3. Multi-Machine Deployment

Same binary, different configs:

**Machine 1: API + WASM agents**
```toml
[queue]
backend = "nats"
nats_url = "nats://nats-server:4222"

[deployment]
mode = "distributed"
harness = true

[workers.agent]
wasm = 4
```

**Machine 2: Docker host**
```toml
[queue]
backend = "nats"
nats_url = "nats://nats-server:4222"

[deployment]
mode = "distributed"
harness = false

[workers.agent]
docker = 8
```

**Machine 3: Inference + remote storage**
```toml
[queue]
backend = "nats"
nats_url = "nats://nats-server:4222"

[deployment]
mode = "distributed"
harness = false

[workers.infer]
ollama = 4

[workers.storage.object]
s3 = 2

[workers.storage.vector]
pinecone = 2
```

Run `vlinder start` on each machine. Config determines role. Only configure backends that are available on that machine.

### 4. Test-Only Backends

Some backends exist for testing, not production:

| Backend | Type | Purpose |
|---------|------|---------|
| Llama.cpp (`llama`) | Inference | Integration tests without Ollama |
| In-memory storage | Object/Vector | Unit tests |
| WASM runtime | Agent | Works, but not hardened |

No special code to exclude them. Convention: don't configure them in production.

```toml
# Development machine
[workers.infer]
llama = 1      # For tests
ollama = 1     # For real work

# Production machine
[workers.infer]
ollama = 4     # Only production backends
# llama not configured
```

If an agent requires a backend that's not configured, routing fails — no workers subscribed to that subject. This is correct behavior: the deployment doesn't support that backend.

### 5. PendingMessage ACK Pattern

Change `MessageQueue::receive()` to return a `PendingMessage` requiring explicit acknowledgment:

```rust
pub trait MessageQueue {
    fn send(&self, queue: &str, msg: Message) -> Result<(), QueueError>;
    fn receive(&self, queue: &str) -> Result<PendingMessage, QueueError>;
}

pub struct PendingMessage {
    pub message: Message,
    ack_fn: Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
    nack_fn: Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
}

impl PendingMessage {
    /// Acknowledge successful processing. Consumes self.
    pub fn ack(self) -> Result<(), QueueError> {
        (self.ack_fn)()
    }

    /// Negative acknowledge. Message redelivered. Consumes self.
    pub fn nack(self) -> Result<(), QueueError> {
        (self.nack_fn)()
    }
}
```

| Backend | `receive()` | `ack()` | `nack()` |
|---------|-------------|---------|----------|
| InMemoryQueue | Removes message | No-op | No-op |
| NatsQueue | Fetches, holds handle | ACKs to JetStream | NAKs, redelivers |

Type safety: `ack()` consumes self — can't double-ACK, compiler warns on unused `PendingMessage`.

### 6. SQLite Concurrent Access

Multiple worker processes access the same SQLite database via WAL mode:

```rust
conn.pragma_update(None, "journal_mode", "WAL")?;
```

- Multiple concurrent readers
- Single writer (others queue)
- All processes on same machine (or shared filesystem)

### 7. Graceful Shutdown

Workers catch `SIGTERM`/`SIGINT`:

```rust
let shutdown = Arc::new(AtomicBool::new(false));
signal_hook::flag::register(SIGTERM, Arc::clone(&shutdown))?;

while !shutdown.load(Ordering::Relaxed) {
    match queue.receive(subject) {
        Ok(pending) => {
            process(&pending.message);
            pending.ack()?;
        }
        Err(QueueError::Timeout) => continue,
    }
}
```

Receive timeout (30s) allows periodic shutdown checks.

### 8. Failure Handling

JetStream handles redelivery:

| Scenario | Behavior |
|----------|----------|
| Worker crashes before ACK | Redelivered after `ack_wait` timeout |
| Worker calls `nack()` | Redelivered immediately |
| Worker calls `ack()` | Message removed |

No application-level retry needed.

## Consequences

**Positive:**
- Horizontal scaling via config
- Process isolation — failures don't cascade
- Same binary everywhere — config determines role
- Backend-specific workers — configure what's available per machine
- Local vs remote distinction — clear mental model for scaling
- Explicit ACK — robust message handling
- Test backends by convention, not code — just don't configure in prod

**Negative:**
- Operational complexity in distributed mode
- Local backends (SQLite, WASM) constrained to single machine
- Breaking change to `receive()` return type
- Subject scheme gains complexity (`vlinder.agent.{runtime}.{name}`)

**Migration:**
- Update all `queue.receive()` callers to handle `PendingMessage`
- Add `.ack()` after processing
- InMemoryQueue unaffected (ACK is no-op)
- Update NATS subjects to include backend type

## Implementation Phases

**Phase 1: PendingMessage pattern**
- Change trait, update both queue implementations
- Update all callers
- No functional change (local mode works as before)

**Phase 2: Deployment config + worker spawning**
- Add `[deployment]` config section
- Add `[workers.*]` config sections (per backend type)
- Daemon reads config, spawns workers
- Update NATS subjects to include backend type

**Phase 3: Multi-machine support**
- `harness = false` option (workers only)
- Documentation for multi-machine setup
- Example configs for common topologies

## Open Questions

- Health checks for workers?
- Metrics/observability for queue depth?
- Dynamic scaling (add workers at runtime)?
