# ADR 046: Agents as OCI Containers via Podman

## Status

Accepted

## Context

Vlinder currently embeds WASM execution in-process via extism. The platform loads the WASM binary, registers host functions (`send`, `get_prompts`), calls the `process` entry point, and manages memory directly. This means Vlinder **is** a WASM runtime — responsible for sandboxing, memory safety, host function ABI stability, and all the plumbing that comes with running untrusted code in-process.

This is not our core value. Our core value is the **message protocol** (ADR 044) and the agent wiring that connects agents to services via queues.

### Current architecture

```
CLI → Daemon → WasmRuntime::tick()
                   ↓
              wasm_plugin::run_plugin()     ← extism in-process
                   ↓
              Plugin::new() + plugin.call("process")
                   ↓
              host fn send() → SendFunctionData::handle_send()
                   ↓
              Queue send/receive
```

The extism coupling shows up in two places:

1. **Platform side** (`wasm_plugin.rs`, 75 lines): `Wasm::file`, `Manifest`, `Plugin::new`, `Function::new`, `UserData`, `CurrentPlugin`, memory management. All extism-specific.

2. **Agent side** (`agents/pensieve/src/host.rs`): `extism_pdk`, `#[host_fn] extern "ExtismHost"`, `unsafe { send(...) }`. The agent is compiled against extism's guest ABI.

### Problems

- We maintain the execution engine instead of focusing on orchestration
- Host function ABI is our problem — any change breaks all compiled agents
- Agents must be Rust compiled to WASM — no other language works
- In-process execution means a misbehaving WASM module can block the runtime thread
- WASM was a means to sandboxed execution, not an end in itself

### Why not WASM-in-containers

Once you're paying the cost of Podman (out-of-process, container startup), running WASM inside crun via WasmEdge gives you:
- Faster startup (milliseconds vs seconds)
- Smaller images (kilobytes vs megabytes)

But you lose:
- **Language freedom** — agents are still WASM-only, still need a compile-to-WASM toolchain
- **Full ecosystem** — no `pip install`, no system libraries, WASI is still maturing
- **Standard tooling** — WASM OCI images are a niche format with limited registry/CI support

For agents that are long-running inference workloads, container startup latency amortizes to nothing. The language and ecosystem constraints matter far more.

## Decision

**Agents are OCI containers, executed by Podman.** WASM is no longer a platform requirement — it becomes one possible language choice for writing an agent.

### Why Podman

- **Daemonless** — fits Vlinder's local-first philosophy (no background Docker daemon)
- **Rootless** — no privilege escalation required
- **OCI standard** — agents are standard container images, publishable to any registry
- **Process isolation** — containers can't block the runtime thread or touch the host
- **Docker-compatible** — standard Dockerfiles, existing CI/CD pipelines just work

### What this enables

Agents can be written in **any language**:
- Python with `pip install` dependencies
- Node.js with npm packages
- Go, Rust, Java — compiled or interpreted
- Even WASM (via a `FROM scratch` image with a `.wasm` entrypoint, if desired)

The platform doesn't care what's inside the container. It only cares about the **transport protocol** between the platform and the agent.

### What changes

**Platform side:**
- `wasm_plugin.rs` is deleted — no more extism imports
- `WasmRuntime` becomes `ContainerRuntime` (or similar)
- `tick()` spawns a Podman container instead of calling `Plugin::new`
- `SendFunctionData::handle_send()` survives unchanged — the bridge logic is runtime-agnostic

**Agent side:**
- Agents no longer use `extism_pdk` or host functions
- Agents communicate with the platform via a transport (deferred — separate ADR)
- Agents are packaged as OCI images with a Dockerfile
- Any language, any dependencies

**Message protocol (ADR 044):**
- Invoke, Request, Response, Complete — unchanged
- Queue subjects — unchanged
- The bridge (`handle_send`) continues to validate SdkMessage, resolve Hop, build RequestMessage, send/receive

### New architecture

```
CLI → Daemon → ContainerRuntime::tick()
                   ↓
              podman run <agent-image>          ← out-of-process
                   ↓
              Agent ←→ Bridge (handle_send)     ← via transport
                   ↓
              Queue send/receive
```

## Consequences

**Positive:**
- Vlinder stops being a WASM runtime and becomes purely an orchestration platform
- Agents can be written in any language
- `wasm_plugin.rs` and `extism` dependency are deleted
- Standard container tooling (Dockerfiles, registries, CI/CD) — nothing new to learn
- Process isolation via Linux namespaces + cgroups (battle-tested)
- Aligns with multi-runtime vision (TODO.md Phase 4) — OCI containers run everywhere

**Negative:**
- Podman becomes a required dependency (must be installed on host)
- Container startup latency per invocation (mitigatable with pooling)
- Pensieve agent must be rewritten for the new communication model
- Agent images are heavier than `.wasm` files (megabytes vs kilobytes)

## Deferred

- Agent-platform transport protocol (HTTP, stdin/stdout, socket — separate ADR)
- Container image build tooling (`vlinder agent build` → OCI image)
- Container pooling for latency-sensitive workloads
- How agent manifests reference OCI images instead of `.wasm` file paths
- `extism` dependency removal from `Cargo.toml`
