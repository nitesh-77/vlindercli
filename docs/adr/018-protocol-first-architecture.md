# ADR 018: Protocol-First Architecture

## Status

Accepted

## Context

The current architecture uses WASM with host function injection (via extism) as the primary interface between agents and runtime. This works well for single-machine deployment and provides excellent sandboxing, but has limitations:

1. **Language restriction**: Only languages that compile to WASM (Rust, C, Zig) can write agents. Python, the dominant AI/ML language, is excluded.

2. **Distributed deployment**: Host function injection is a same-process mechanism. It doesn't work when agents and runtime are on separate machines.

3. **Rich runtime vision**: We want the runtime to provide rich capabilities to the agents. These need to be accessible to all agent implementations, not just WASM.

The tension: WASM is ideal for single-machine use (lightweight, portable, sandboxed), but a WASM-only approach excludes most developers and prevents distributed deployment.

## Decision

### The Runtime Protocol is the Primary Abstraction

WASM host functions are one implementation of that protocol, not the protocol itself.

The protocol defines the vocabulary of operations agents can perform: `infer`, `embed`, `store`, `search`, `call_agent`, etc.  This inversion is important: we don't design host functions and then expose them to the agents. 

### Core Insight: Queue as Universal Abstraction

The protocol is fundamentally about message passing. Every operation—inference, embedding, storage, agent-to-agent calls—is a message sent to a queue and a response received.

```
┌─────────────────────────────────────────────────────────────┐
│                        Message Queue                        │
│                   (in-process / Redis / SQS)                │
│                                                             │
│   ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐           │
│   │inference│ │embedding│ │ storage │ │ agents  │           │
│   │ service │ │ service │ │ service │ │         │           │
│   └─────────┘ └─────────┘ └─────────┘ └─────────┘           │
│        ▲           ▲           ▲           ▲                │
│        └───────────┴───────────┴───────────┘                │
│                         │                                   │
│                    all just workers                         │
│                    all just messages                        │
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

### Determinism

Local-first enables determinism at the *event boundaries*.

Because LLM inference runs on parallel hardware with floating-point math and scheduling variability, bit-for-bit determinism is not always possible. VlinderCLI instead guarantees determinism at the system boundary: executions are reproducible from their recorded events.

- **Local models**: Models are versioned, hash-verified artifacts. We pin exact weights, runtimes, and inference configs so executions are reproducible and comparable across runs.

- **Message log**: All messages and external interactions are persisted with provenance (inputs, outputs, model/version/config, and references to retrieved data). This creates a complete audit trail.

**Replay any execution:**
Given the same message log, model versions, inference settings, and storage snapshots, VlinderCLI can replay executions to produce the same *observable behavior*.

This is operational reproducibility.
API-based systems can change models or behavior server-side without visibility.
With local models and event logs, you can verify, replay, and audit your system end-to-end.

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

## Consequences

- **One abstraction**: Queue-based message passing unifies everything.

- **Deterministic at event boundaries**: Local models + message log = reproducible execution.

- **Scale anything**: Add workers to any queue. No architectural changes.

## See Also

- **ADR 030**: Domain module structure — where service workers live and why they're domain entities.
