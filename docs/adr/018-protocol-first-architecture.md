# ADR 018: Protocol-First Architecture

## Status

Proposed

## Context

The current architecture uses WASM with host function injection (via extism) as the primary interface between agents and runtime. This works well for single-machine deployment and provides excellent sandboxing, but has limitations:

1. **Language restriction**: Only languages that compile to WASM (Rust, C, Zig) can write agents. Python, the dominant AI/ML language, is excluded.

2. **Distributed deployment**: Host function injection is a same-process mechanism. It doesn't work when agents and runtime are on separate machines.

3. **Rich runtime vision**: We want the runtime to provide higher-order functions (chain of thought, tree of thought, query expansion, multi-hop retrieval). These need to be accessible to all agent implementations, not just WASM.

The tension: WASM is ideal for single-machine use (lightweight, portable, sandboxed), but a WASM-only approach excludes most developers and prevents distributed deployment.

## Decision

### Design Philosophy: One Computer

Design the system as if it runs on one computer.

When you need multiple computers to act as one, there are well-understood solutions: message queues, distributed storage, load balancers. These have trade-offs (CAP theorem, eventual consistency), but the decisions are accepted and the infrastructure is commodity.

The single-machine version is the system, not a toy. Local WASM execution uses the same protocol, same types, same SDK as distributed deployment. The only difference is the underlying infrastructure:

| Abstraction | Local | Distributed |
|-------------|-------|-------------|
| Queue | In-process | Redis, SQS, Kafka |
| Storage | SQLite | S3, DynamoDB |
| Compute | WASM | Containers, Lambda |

Scaling means scaling the infrastructure, not rewriting the application. The protocol stays stable. The SDK interface stays stable. The types stay stable.

**Well-designed agents scale without code changes.** This requires agents to follow good design principles:

- **Ports and adapters**: SDK calls at the boundaries, core logic isolated
- **Domain-driven design**: Business logic doesn't know about infrastructure

If agents mix infrastructure concerns into core logic, they may need changes when scaling. The system makes good design easy—SDK as explicit dependency, testable without infrastructure, templates that show the right patterns—but can't force it.

### The Runtime Protocol is the Primary Abstraction

WASM host functions are one implementation of that protocol, not the protocol itself.

The protocol defines the vocabulary of operations agents can perform: `infer`, `embed`, `store`, `search`, `call_agent`, etc. How these operations are delivered—host functions, HTTP, gRPC, message queues—is an implementation detail.

This inversion is important: we don't design host functions and then expose them elsewhere. We design the protocol and then implement it across transports. The protocol is the source of truth.

### Core Insight: Queue as Universal Abstraction

The protocol is fundamentally about message passing. Every operation—inference, embedding, storage, agent-to-agent calls—is a message sent to a queue and a response received.

```
┌─────────────────────────────────────────────────────────────┐
│                        Message Queue                         │
│                   (in-process / Redis / SQS)                 │
│                                                              │
│   ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐          │
│   │inference│ │embedding│ │ storage │ │ agents  │          │
│   │ service │ │ service │ │ service │ │         │          │
│   └─────────┘ └─────────┘ └─────────┘ └─────────┘          │
│        ▲           ▲           ▲           ▲                │
│        └───────────┴───────────┴───────────┘                │
│                         │                                    │
│                    all just workers                          │
│                    all just messages                         │
└─────────────────────────────────────────────────────────────┘
```

**Everything is a service.** Inference, embedding, storage, and agents are all queue workers. There's no distinction at the protocol level—they all consume messages and produce responses.

**Queue implementation varies by deployment:**

| Deployment | Queue Implementation |
|------------|---------------------|
| WASM single-process | In-process (direct calls) |
| Local containers | Local queue (Redis, in-memory) |
| Distributed | Cloud queue (SQS, Kafka, NATS) |
| Lambda | SQS triggers |

Same protocol. Same message types. Different queue backend.

### Typed Messages as Source of Truth

Rust types define the protocol. Each operation has typed request/response messages:

```rust
// Inference
pub struct Phi3InferRequest {
    pub prompt: String,
    pub temperature: Temperature,  // validated range
    pub max_tokens: MaxTokens,     // model-specific limit
}

pub struct InferResponse {
    pub text: String,
}

// Embedding
pub struct NomicEmbedRequest {
    pub text: String,
    pub task_type: NomicTaskType,  // enum: SearchQuery, SearchDocument, etc.
}

pub struct EmbedResponse {
    pub vector: Vec<f32>,
}

// Agent-to-agent
pub struct AgentRequest {
    pub input: String,
}

pub struct AgentResponse {
    pub output: String,
}
```

**Types encode best practices.** Each model/engine has specific types with validated parameters. Users can't construct invalid requests—the compiler prevents it.

**Types are pedagogy.** To use a type, you must see its fields. To see the fields, you start to understand them. Types force intentionality.

### SDK as Thin Wrapper

The SDK is minimal—just queue operations with typed wrappers:

```rust
impl Sdk {
    // Core: publish and receive
    fn publish(&self, target: &str, msg: impl Message) -> Result<()>;
    fn receive(&self) -> Result<Message>;

    // Typed convenience wrappers
    fn infer(&self, req: Phi3InferRequest) -> InferResponse;
    fn embed(&self, req: NomicEmbedRequest) -> EmbedResponse;
    fn call_agent(&self, agent: &str, req: AgentRequest) -> AgentResponse;
}
```

Same SDK interface regardless of deployment. The queue backend is injected.

### Determinism

Local-first + typed messages = fully deterministic system.

- **Local models**: Same model file + same input = same output. Every time.
- **Typed messages**: Validated at compile time. Can't send malformed requests.
- **Message log**: Every operation logged. Full audit trail.

**Replay any execution:** Same messages + same model files + same storage snapshot = identical behavior.

This is reproducibility that API-based systems (OpenAI, Anthropic) cannot offer. They have server-side variability. We have math.

### Scaling

Scale any service by adding workers:

```
agent-a-queue
     │
     ├──► agent-a instance 1
     ├──► agent-a instance 2
     └──► agent-a instance 3
```

Queue handles distribution. No coordinator in the data path. No bottleneck.

This applies to all services:
- Need more inference throughput? Add inference workers.
- Need more embedding throughput? Add embedding workers.
- Need more agent capacity? Add agent instances.

### Control Plane vs Data Plane

**Data plane (queue):**
- Message routing
- Load balancing
- Scaling

**Control plane (vlinderd):**
- Agent/Fleet registry
- Worker health
- Configuration

vlinderd is NOT in the data path. Pure control plane.

### Storage is Bytes

Storage (object and vector) stores bytes. No schema enforcement.

```rust
PutFileRequest { path: String, content: Vec<u8> }  // bytes
StoreEmbeddingRequest { key: String, vector: Vec<f32>, metadata: String }  // string
```

Protocol is typed. User data is user's responsibility. Don't boil the ocean.

## WASM Value Proposition

WASM remains valuable for specific use cases:

- **Sandboxing**: Capability-based isolation. Safe to run untrusted code.
- **Zero-overhead local execution**: In-process queue = direct function calls.
- **Portability**: One binary runs on any OS/architecture.
- **Fast startup**: ~1-5ms vs containers at ~500ms+.
- **Community marketplace**: Safe to download and run third-party agents.

WASM is one executor. The protocol works across all executors (WASM, Podman, Firecracker, Lambda).

## Consequences

- **One abstraction**: Queue-based message passing unifies everything.

- **Types are the protocol**: Rust types define valid operations. SDKs generated from types.

- **Deterministic**: Local models + typed messages + message log = reproducible execution.

- **Scale anything**: Add workers to any queue. No architectural changes.

- **Control/data separation**: vlinderd for control, queues for data. No bottleneck.

- **Backward compatibility freedom**: No API to maintain forever. Add new types for new models.

- **Python can't compete**: Compile-time guarantees, determinism, and reproducibility are architectural—not features you can add to stringly-typed systems.

## Open Questions (TODO)

### vlinderd (Control Plane)

Not fully designed. Questions to resolve:

- What exactly does vlinderd manage? (Registry, health, config, queue provisioning?)
- Does discovery matter if queues handle routing by name?
- How does lifecycle management work? (start/stop agents/fleets)
- Worker registration model?
- State persistence across restarts?

### Agent SDK

Not fully designed. Questions to resolve:

- How does SDK discover queue endpoint?
- Environment injection? Config file? Service discovery?
- SDK for WASM vs SDK for containers—same interface, different internals?

### Message Envelope

Need to finalize message structure:

```rust
struct Message {
    target: String,
    payload: Vec<u8>,  // or typed?
    reply_to: String,
    correlation_id: String,
}
```

- Binary vs JSON payload?
- How to handle streaming responses?
- Error representation in responses?

## Future ADRs

This ADR is the vision. Implementation will be validated through incremental ADRs:

- ADR 0XX: MessageQueue trait + InProcess implementation
- ADR 0XX: Typed inference messages (Phi3, Llama3, etc.)
- ADR 0XX: Agent-to-agent via queue
- ADR 0XX: vlinderd control plane design
- ADR 0XX: Redis/SQS queue implementations

ADR 018 moves to "Accepted" when the vision is validated through implementation.
