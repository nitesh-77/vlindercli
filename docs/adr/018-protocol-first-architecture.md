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

**The runtime protocol is the primary abstraction. WASM host functions are one implementation of that protocol.**

The vocabulary we expose (`infer`, `embed`, `chain_of_thought`, `search_by_vector`, etc.) is defined as a protocol specification. Multiple transports implement this protocol:

| Transport | Best For | Overhead |
|-----------|----------|----------|
| WASM host functions | Sandboxed agents, single machine | Near-zero (in-process) |
| Unix socket | Local SDKs (Python), single machine | Minimal (IPC) |
| HTTP | Universal access, debugging, any language | Moderate |
| gRPC | Distributed systems, high throughput | Efficient binary |

**Implementation: Rust trait as source of truth.**

A Rust trait defines the protocol. Both WASM host functions and HTTP/gRPC endpoints implement this trait, ensuring identical behavior. OpenAPI is derived from the trait (via utoipa) for SDK generation.

```
┌─────────────────────────────────────────────────────────┐
│               RUST TRAIT (source of truth)              │
│                                                         │
│   pub trait VlinderProtocol {                           │
│       fn infer(&self, req: InferRequest) -> Response;   │
│       fn embed(&self, req: EmbedRequest) -> Embedding;  │
│       fn chain_of_thought(&self, ...) -> Response;      │
│       fn search_by_vector(&self, ...) -> Vec<Document>; │
│       ...                                               │
│   }                                                     │
└─────────────────────────────────────────────────────────┘
                          │
       ┌──────────────────┼──────────────────┐
       ▼                  ▼                  ▼
┌─────────────┐   ┌─────────────┐   ┌─────────────────┐
│ impl for    │   │ impl for    │   │ utoipa/OpenAPI  │
│ WasmRuntime │   │ HttpServer  │   │ → Generated SDKs│
└─────────────┘   └─────────────┘   └─────────────────┘
```

This approach enables fast iteration while the protocol evolves. The compiler enforces consistency—adding a method to the trait requires both implementations to update. SDKs (Python, Go, etc.) are generated from the derived OpenAPI spec via AutoRest or OpenAPI Generator.

**WASM provides unique value:**

- **Sandboxing**: Capability-based isolation. Agents can only access what's explicitly provided. Safe to run untrusted code.
- **Zero-overhead local execution**: No serialization, no IPC. Direct function calls.
- **Portability**: One binary runs on any OS/architecture.
- **Resource limits**: Enforce memory, fuel (instruction count), and time limits.
- **No runtime dependencies**: No Python/Node/JVM installation required.
- **Fast startup**: ~1-5ms vs containers at ~500ms+.

WASM is the recommended path for:
- Untrusted/third-party agents (sandbox)
- Community agent marketplace (safe to download and run)
- Single-machine deployment without Docker/K8s overhead
- Maximum local performance

SDKs (Python, etc.) are the recommended path for:
- Developers iterating quickly in their preferred language
- Distributed deployments where agents run on separate machines
- Environments with existing language/tooling standards

## Consequences

- **Single source of truth**: The Rust trait defines the protocol. OpenAPI and SDKs are derived, not manually maintained.

- **WASM and SDKs serve different trust models**: WASM for untrusted code (sandboxed), SDKs for trusted code (full host access).

- **Same vocabulary everywhere**: A developer learns `chain_of_thought` once, uses it from WASM, Python, or curl.

- **Single-machine deployment stays simple**: WASM + single binary runtime. No containers, no orchestration, no network stack.

- **Distributed deployment becomes a transport change**: Same protocol, different transport. Not a rewrite.

- **Higher-order functions are protocol operations**: `chain_of_thought`, `tree_of_thought`, `query_expand` are verbs in the vocabulary, available to all transports.

- **Future work**: Define and stabilize the protocol spec before implementing additional transports.
