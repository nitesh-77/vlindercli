# ADR 031: vlinderd as Runtime Registry

## Status

Proposed

## Context

ADR 018 established queue-based message passing. ADR 029 defined harnesses as user-facing interfaces.

The current `CliHarness` embeds `WasmRuntime` directly — it creates and manages the runtime in-process. This coupling was useful for learning (see ADR 029) but now limits us: harnesses should work with any runtime, local or remote.

## Decision

**vlinderd is the central registry and runtime manager.**

### Harness is a Thin Client

Harnesses (CLI, Web, API) do NOT create runtimes. They:

1. Ask vlinderd for discovery (agents, models, runtimes)
2. Get runtime connection info from vlinderd
3. Communicate with runtimes via the provided interface

```
CliHarness
    │
    ├─► vlinderd: "resolve agent 'pensieve'"
    │   └─► { runtime: "wasm-local", endpoint: "unix:///var/run/vlinder/wasm.sock" }
    │
    └─► Runtime (via socket)
        └─► executes agent, returns response
```

### vlinderd Manages Runtime Lifecycle

vlinderd boots, monitors, and provides endpoints for all runtimes:

```
vlinderd
    │
    ├── boots WasmRuntime (local process, unix socket)
    ├── connects to AwsRuntime (Lambda API)
    ├── connects to K8sRuntime (K8s API)
    │
    └── exposes discovery API
        ├── GET /runtimes → list available runtimes
        ├── GET /agents/{name} → { runtime, endpoint, status }
        └── POST /agents/{name}/invoke → proxied or direct?
```

### Runtime Interface

All runtimes expose the same interface, transport varies:

| Runtime | Transport | Endpoint Example |
|---------|-----------|------------------|
| WasmRuntime | Unix socket | `unix:///var/run/vlinder/wasm.sock` |
| WasmRuntime | TCP | `tcp://localhost:9000` |
| AwsRuntime | HTTPS | `https://lambda.us-east-1.amazonaws.com` |
| K8sRuntime | HTTPS | `https://k8s.cluster.local` |

The harness doesn't care — it gets an endpoint from vlinderd and talks to it.

### Runtime Trait

```rust
pub trait Runtime: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> RuntimeCapabilities;

    // Agent lifecycle
    fn create(&self, agent: &Agent) -> Result<(), RuntimeError>;
    fn delete(&self, agent_name: &str) -> Result<(), RuntimeError>;
    fn status(&self, agent_name: &str) -> Result<AgentStatus, RuntimeError>;
}

pub struct RuntimeCapabilities {
    pub executor: ExecutorType,  // wasm, lambda, container
    pub models: Vec<String>,     // available models
    pub gpu: bool,
}
```

### The Flow

vlinderd is the vlindercli daemon.
  - it is what provides the entire functionality of this software.
  - it runs as a service that spawns all the processes require to provide the functionality.
  - it provides discovery as a service over http -- one of the processes.
  - it runs all the runtimes (local wasm, aws, k8s.... whichever is enabled, and listens to respective
  queues)
  - it kicks off a message queue (or acts as the local proxy for when the message queue is on the
  cloud)

  harness cli when run by a user from a terminal,
  - accepts user inputs
  - reads manifests from current directory
  - registers them (this is an idempotent operation - create if it doesn't exist, read if it does)
  - reads queue info from registry
  - creates a message to run the agent, based on manifests in current dir
  - queues up the message in the queue for the runtime to consume

  runtime, having been started by vlinderd
  - consumes the message in the message queue
  - inspects the message info, which contains everything it needs to invoke the agent
  - invokes the agent
  - if the agents deployment target is wasm, it laumches the wasm files
  - if deployment target is lambdas/podman/firecracker, it invokes them

  the agent
  - if it has finished executing, queues up a message in the runtime's reply queue
  - the runtime then queues up the response back to the harness's reply-to.

  if it needs any access to the services (vector, object, inference, embedding)
  - makes the request known to runtime (push/pull, based on type)

  the runtime
  - queues up the message to the respective services (vector, object, inference, embedding)

  the services (having been started as processes by vlinderd)
  - listens to messages in the queue
  - invokes respective systems (dynamodb/s3/pinecone/bedrock/.....)
  - gathers responses
  - queues up a message back to the reply-to queue (the agent's queue)

  THis is the desired state. Which part of this is not clear to you? Ask if you have questions.


### What vlinderd Tracks

```
vlinderd
    │
    ├── Runtimes
    │   ├── wasm-local (spawned by vlinderd, unix socket)
    │   ├── aws-prod (configured, HTTPS to Lambda)
    │   └── k8s-dev (configured, HTTPS to cluster)
    │
    ├── Models
    │   └── phi3, nomic-embed, ... (paths, capabilities)
    │
    ├── Agents
    │   └── pensieve → { runtime: wasm-local, status: running }
    │
    └── Fleets
        └── my-fleet → { agents: [...], status: deployed }
```

## Current State vs Desired State

### Current State

```
CliHarness (single process)
├── embeds WasmRuntime
├── embeds Provider (object, vector, inference, embedding)
├── embeds InMemoryQueue
└── register() does everything: creates storage, loads models, registers agent
```

Everything is in-process. Harness IS the system.

**Domain entities implemented:**

| Entity | Status | Notes |
|--------|--------|-------|
| ResourceId | ✓ | URI-based key for registry lookups |
| ObjectStorageManifest | ✓ | Serde-deserializable, tagged enum |
| VectorStorageManifest | ✓ | Serde-deserializable, tagged enum |
| AgentManifest.object_storage | ✓ | Optional ResourceId field |
| AgentManifest.vector_storage | ✓ | Optional ResourceId field |
| Registry | ✓ | Stores jobs, agents, and available runtimes |
| JobId | ✓ | Full URI: `<registry_id>/jobs/<uuid>` |
| Job | ✓ | agent_id (ResourceId), input, status |
| JobStatus | ✓ | Pending, Running, Completed, Failed |
| Daemon | ✓ | Control plane owning Registry, Harness, Runtime, Provider |
| Harness (Daemon-owned) | ✓ | API surface for invoke/poll, owned by Daemon |
| Runtime uses ResourceId | ✓ | Agents keyed by ResourceId, queue routing by agent_id |
| RuntimeType | ✓ | Compile-time enum: Wasm (future: Lambda, Container) |
| Runtime.id() | ✓ | Returns ResourceId: `<registry_id>/runtimes/<type>` |
| Runtime dispatch | ✓ | Registry.select_runtime() - see ADR 034 |

### Desired State

```
vlinderd (daemon) ────spawns────┬── discovery service (HTTP)
                                ├── message queue
                                ├── runtimes (wasm, aws, k8s)
                                └── services (object, vector, inference, embedding)

harness (thin client) ──────────┬── reads manifests
                                ├── registers with vlinderd
                                ├── gets queue info from registry
                                └── queues message
```

All separate processes communicating via queues.

### Key Gaps

| Component | Current | Desired |
|-----------|---------|---------|
| vlinderd | **Daemon exists** (in-process) | spawns all processes |
| registry | **Registry exists** (in-process) | HTTP discovery service |
| harness | **Harness owned by Daemon** | thin client (separate process) |
| runtime | embedded in Daemon | separate process |
| services | embedded in Daemon | separate processes |
| queue | in-memory, in-process | separate (or cloud proxy) |

**Progress**: Core abstractions (Daemon, Registry, Harness, Job) are implemented in-process. Next step is process separation.

### ResourceId

All resources in the registry are identified by URI:

```rust
pub struct ResourceId(String);
```

The scheme encodes the resource type:
- `sqlite:///path/to/db.sqlite` → SQLite storage
- `s3://bucket/prefix` → S3 storage
- `memory://test` → in-memory (testing)
- `ollama://phi3` → Ollama model
- `file:///path/to/model.gguf` → local model file

ResourceId is:
- The key for registry lookups (`HashMap<ResourceId, Arc<dyn ObjectStorage>>`)
- Declared in manifests (`object_storage = "sqlite:///data/notes.db"`)
- Resolved by harness, looked up by runtime

**Implemented:** `AgentManifest` now has optional storage fields:

```rust
pub struct AgentManifest {
    // ...
    pub object_storage: Option<ResourceId>,
    pub vector_storage: Option<ResourceId>,
}
```

Example agent.toml:
```toml
name = "pensieve"
description = "Memory agent"
code = "agent.wasm"
object_storage = "sqlite:///data/objects.db"
vector_storage = "sqlite:///data/vectors.db"

[requirements]
services = ["inference", "embedding"]
```

## Consequences

- **Harness is decoupled**: No embedded runtime, just a thin client
- **Uniform interface**: Same harness code works with any runtime
- **vlinderd is the authority**: Single source of truth for what's running where
- **Runtimes are processes**: Even local WasmRuntime is a separate process managed by vlinderd
- **Testability**: Can mock vlinderd responses for testing harnesses

## Migration

Current `CliHarness` embeds `WasmRuntime`. Migration path:

1. Extract WasmRuntime to separate process with socket interface
2. Add vlinderd with runtime registry
3. Update CliHarness to query vlinderd instead of embedding runtime
4. Existing tests use `TestHarness` that embeds everything (no vlinderd needed)

## Open Questions

- Protocol for harness ↔ runtime communication (gRPC? HTTP? custom?)
- vlinderd API design (REST? gRPC?)
- Runtime health checks and automatic restart
- Local development mode (vlinderd + WasmRuntime in one process?)
