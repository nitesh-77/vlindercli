# ADR 043: Multi-Processing Architecture

## Status

Accepted

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
[distributed]
enabled = false  # Default: local mode
```

| Mode | Queue | Execution | Use Case |
|------|-------|-----------|----------|
| Local (`enabled = false`) | memory or nats | Single process, tick-based loop | Development, testing |
| Distributed (`enabled = true`) | nats (required) | Separate processes per service | Production, scaling |

**Local mode**: Daemon runs all services in a single process via sequential `tick()` calls. CLI commands like `vlinder agent run` block and wait for results.

**Distributed mode**: Daemon runs as a system service, spawning worker processes. CLI commands (`vlinder agent deploy`, `vlinder agent run`) are separate processes that queue messages to NATS. Workers pick up messages and process them. No `harness.tick()` needed — the CLI is the client, workers are the servers, NATS is the broker.

### 2. Configuration

Each count specifies the number of **separate OS processes** to spawn for that service type. These are not threads — they are independent processes with their own memory space, communicating via NATS (queue) and gRPC (registry).

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

# Distributed mode configuration
# Omit or set enabled = false for local (single-process) mode
[distributed]
enabled = true
registry_addr = "http://127.0.0.1:9090"

# Registry service — runs the gRPC registry server (ADR 042)
# count = 1 means this machine hosts the registry
# count = 0 (or omitted) means connect to registry_addr as client
[distributed.registry]
count = 1

# Agent runtimes — each count = number of OS processes
[distributed.agent]
wasm = 4              # 4 WASM executor processes
docker = 2            # 2 Docker runtime processes

# Inference engines — each count = number of OS processes
[distributed.inference]
ollama = 2            # 2 Ollama proxy processes

# Embedding engines — each count = number of OS processes
[distributed.embedding]
ollama = 1            # 1 Ollama embedding process

# Object storage — each count = number of OS processes
[distributed.storage.object]
sqlite = 1            # 1 SQLite storage process

# Vector storage — each count = number of OS processes
[distributed.storage.vector]
sqlite-vec = 1        # 1 SQLite-vec process
```

**Key insight:** Backends fall into two categories:

| Type | Examples | Characteristics |
|------|----------|-----------------|
| Local execution | WASM, Docker, Firecracker, SQLite | Runs on worker machine, resource constrained |
| Remote proxy | Ollama, S3, Pinecone, OpenAI | Stateless HTTP, scale freely |

Local backends have deployment constraints. Remote backends are just proxies — scale by adding workers.

NATS subjects include backend type for routing (see ADR 044 for typed message subjects):
```
vlinder.{sub}.invoke.{harness}.{runtime}.{agent}
vlinder.{sub}.req.{agent}.{service}.{backend}.{operation}.{seq}
vlinder.{sub}.res.{service}.{backend}.{agent}.{operation}.{seq}
vlinder.{sub}.complete.{agent}.{harness}
```

Workers subscribe to backends they can handle using wildcard filters (e.g., `vlinder.*.req.*.infer.ollama.*.*`). A machine without a backend simply has no workers subscribing to its subjects.

### 3. Multi-Machine Deployment

Same binary, different configs:

**Machine 1: Registry + WASM agents**
```toml
[queue]
backend = "nats"
nats_url = "nats://nats-server:4222"

[distributed]
enabled = true
registry_addr = "http://0.0.0.0:9090"

[distributed.registry]
count = 1             # This machine hosts the registry

[distributed.agent]
wasm = 4
```

**Machine 2: Docker host**
```toml
[queue]
backend = "nats"
nats_url = "nats://nats-server:4222"

[distributed]
enabled = true
registry_addr = "http://machine1:9090"  # Connect to Machine 1's registry

# No [distributed.registry] = connects as client

[distributed.agent]
docker = 8
```

**Machine 3: Inference + remote storage**
```toml
[queue]
backend = "nats"
nats_url = "nats://nats-server:4222"

[distributed]
enabled = true
registry_addr = "http://machine1:9090"

[distributed.inference]
ollama = 4

[distributed.storage.object]
s3 = 2

[distributed.storage.vector]
pinecone = 2
```

Run `vlinder start` on each machine. Config determines role. Only configure services that are available on that machine.

### 4. Test-Only Backends

Some backends exist for testing, not production:

| Backend | Type | Purpose |
|---------|------|---------|
| Llama.cpp (`llama`) | Inference | Integration tests without Ollama |
| In-memory storage | Object/Vector | Unit tests |
| WASM runtime | Agent | Works, but not hardened |

No special code to exclude them. Convention: don't configure them in production.

```toml
# Development machine (runs everything locally)
[distributed]
enabled = true
registry_addr = "http://127.0.0.1:9090"

[distributed.registry]
count = 1

[distributed.inference]
llama = 1      # For tests
ollama = 1     # For real work

# Production machine (connects to shared registry)
[distributed]
enabled = true
registry_addr = "http://registry:9090"

[distributed.inference]
ollama = 4     # Only production backends
```

If an agent requires a backend that's not configured, routing fails — no workers subscribed to that subject. This is correct behavior: the deployment doesn't support that backend.

### 5. ACK Pattern

Typed receive methods (ADR 044) return a tuple of `(TypedMessage, ack_fn)` requiring explicit acknowledgment:

| Backend | `receive_*()` | `ack()` |
|---------|---------------|---------|
| InMemoryQueue | Removes message | No-op |
| NatsQueue | Fetches from JetStream consumer | ACKs to JetStream |

Workers must call `ack()` after processing to remove the message from the WorkQueue stream.

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
- Subject scheme gains complexity (see ADR 044 for typed message subjects)

**Migration:**
- Use typed send/receive methods (ADR 044) with explicit `ack()` after processing
- Update NATS subjects to include backend type

## Open Questions

- Health checks for workers?
- Metrics/observability for queue depth?
- Dynamic scaling (add workers at runtime)?
