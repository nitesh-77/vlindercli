# ADR 071: Message Diagnostics and Service Identity

## Status

Proposed

## Context

ADR 067 says every NATS message is a DAG node. ADR 069 says the message sender is the git commit author. ADR 070 says each commit snapshots the system state in a rich tree.

Three gaps remain:

**No diagnostics.** Messages carry a payload but no metadata about the execution that produced them. An inference response doesn't tell you how many tokens were used. A container completion doesn't include stderr. A storage response doesn't report latency. The git commit tree has the answer — but not the context that produced it.

**Services have no identity.** ADR 069 established agent identity: `support-agent <support-agent@registry.local:9000>`. But services — inference engines, storage backends — are also registry entries. When Ollama responds to an inference request, the commit author is just `infer.ollama` with no registry-backed identity. Services should be first-class identities, same as agents.

**Session-id doesn't propagate.** Only Invoke and Delegate messages carry `session-id` in their NATS headers. Request, Response, and Complete messages don't, so the DagCaptureWorker drops them. All five message types need session-id for the git projection to be complete.

### What we control

We own the agent contract (HTTP bridge), the message types (domain model), the service workers, and the container runtime. The entire pipeline from agent execution through NATS to git is ours.

### Provenance: the platform sets diagnostics, not the agent

Diagnostics follow the same provenance model as identity (ADR 069). The platform component that emits a message populates its diagnostics — not the agent, not the external service. The agent never touches the diagnostics struct directly.

| Message | Emitter (platform component) | What it populates |
|---------|------------------------------|-------------------|
| Invoke | Harness | Version, history turns |
| Request | Container Runtime (HTTP bridge) | Sequence, endpoint, request size, timing |
| Response | Service Worker | Tokens, latency, backend metrics |
| Complete | Container Runtime | Podman info, duration, stderr |
| Delegate | Container Runtime | Same as Complete |

The agent returns an HTTP response with a body and stderr. The **container runtime** reads both from the HTTP response, wraps them with platform-level metadata (Podman version, image digest, wall-clock timing), and emits the `CompleteMessage` with `ContainerDiagnostics` fully populated. The agent cannot inflate token counts or misreport its container version — it never sees those fields.

Similarly, the `InferenceServiceWorker` calls Ollama, measures wall-clock duration, extracts token counts from Ollama's response, and emits the `ResponseMessage` with `ServiceDiagnostics`. Ollama doesn't know about our diagnostics model.

This makes the git commit tree a **verified audit record**, not a self-reported log.

## Decision

### 1. Strongly-typed diagnostics per message type

Each message type carries diagnostics specific to its emitter. No shared `Diagnostics` struct with optional fields — the type system encodes what each platform component can guarantee.

```rust
pub struct CompleteMessage {
    // ... existing fields
    pub diagnostics: ContainerDiagnostics,
}

pub struct ResponseMessage {
    // ... existing fields
    pub diagnostics: ServiceDiagnostics,
}

pub struct InvokeMessage {
    // ... existing fields
    pub diagnostics: InvokeDiagnostics,
}

pub struct RequestMessage {
    // ... existing fields
    pub diagnostics: RequestDiagnostics,
}

pub struct DelegateMessage {
    // ... existing fields
    pub diagnostics: DelegateDiagnostics,
}
```

#### ContainerDiagnostics (Complete, Delegate)

**Emitter: Container Runtime** (`ContainerRuntime` in `src/runtime/container.rs`).

The container runtime already holds all the information needed:
- **stderr**: Read from the agent's HTTP response (see agent contract below)
- **engine_version**: Cached at runtime startup via `podman version`
- **image_ref**: From the agent's `executable` field in the registry
- **image_digest**: From `podman inspect <image>` at container start
- **container_id**: Returned by `podman run`
- **duration_ms**: Wall-clock time measured by the runtime around the HTTP POST to `/invoke`

The agent never sees or populates these fields. The runtime reads the agent's HTTP response, extracts `stderr`, and wraps everything with platform metadata.

```rust
pub struct ContainerDiagnostics {
    /// Agent's stderr stream, extracted from the HTTP response by the runtime.
    pub stderr: Vec<u8>,
    /// Container runtime metadata, populated entirely by the platform.
    pub runtime: ContainerRuntimeInfo,
    /// Wall-clock execution time measured by the runtime.
    pub duration_ms: u64,
}

pub struct ContainerRuntimeInfo {
    /// Podman engine version (e.g., "5.3.1"). Cached at startup.
    pub engine_version: String,
    /// OCI image reference (e.g., "localhost/echo-agent:latest"). From registry.
    pub image_ref: String,
    /// Image digest (e.g., "sha256:a80c4f17..."). From podman inspect.
    pub image_digest: String,
    /// Container ID for this execution. From podman run.
    pub container_id: String,
}
```

**Agent contract evolution.** The HTTP bridge response format changes from raw bytes to structured JSON. The runtime parses this; the agent just returns what it has:

```json
{
  "response": "The article discusses...",
  "stderr": "INFO: loaded model in 2.3s\nWARN: context truncated"
}
```

The `response` field becomes the `CompleteMessage.payload`. The `stderr` field becomes `ContainerDiagnostics.stderr`. Everything else in the diagnostics is populated by the runtime, not the agent.

#### ServiceDiagnostics (Response)

**Emitter: Service Workers** (`InferenceServiceWorker`, `EmbeddingServiceWorker`, `ObjectServiceWorker`, `VectorServiceWorker`).

Each service worker calls its backend (Ollama, OpenRouter, SQLite), measures duration, and extracts metrics from the backend's response. The external service doesn't know about our diagnostics model.

```rust
pub struct ServiceDiagnostics {
    /// Service type: "infer", "embed", "kv", "vec".
    pub service: String,
    /// Backend identifier: "ollama", "openrouter", "sqlite", "memory".
    pub backend: String,
    /// Execution time in milliseconds.
    pub duration_ms: u64,
    /// Service-specific metrics.
    pub metrics: ServiceMetrics,
}

pub enum ServiceMetrics {
    Inference {
        tokens_input: u32,
        tokens_output: u32,
        model: String,
    },
    Embedding {
        dimensions: u32,
        model: String,
    },
    Storage {
        operation: String,
        bytes_transferred: u64,
    },
}
```

#### InvokeDiagnostics (Invoke)

**Emitter: Harness** (`CliHarness` in `src/domain/harness.rs`).

```rust
pub struct InvokeDiagnostics {
    /// Harness version.
    pub harness_version: String,
    /// Number of history entries included in payload.
    pub history_turns: u32,
}
```

#### RequestDiagnostics (Request)

**Emitter: Container Runtime (HTTP bridge)** — when the agent makes an SDK call, the bridge intercepts it and emits the Request.

The bridge sits between the agent and every service. It sees every outbound call the agent makes — what it asked for, how much data it sent, and when. This is platform-observed system state.

```rust
pub struct RequestDiagnostics {
    /// Sequence number within the submission.
    pub sequence: u32,
    /// The bridge endpoint the agent called (e.g., "/infer", "/kv/get").
    pub endpoint: String,
    /// Size of the agent's request body in bytes.
    pub request_bytes: u64,
    /// Timestamp when the bridge received the call (Unix millis).
    pub received_at_ms: u64,
}
```

#### DelegateDiagnostics (Delegate)

**Emitter: Container Runtime (HTTP bridge)** — when the agent delegates to another agent via the SDK.

```rust
pub struct DelegateDiagnostics {
    /// Same as ContainerDiagnostics — delegation involves container execution.
    pub container: ContainerDiagnostics,
}
```

### 2. Service identity in the registry

Services are registry entries with identities that follow the same format as agents (ADR 069).

| Entity | Identity | Email |
|--------|----------|-------|
| Agent | `support-agent` | `support-agent@registry.local:9000` |
| Inference | `infer.ollama` | `infer.ollama@registry.local:9000` |
| Embedding | `embed.ollama` | `embed.ollama@registry.local:9000` |
| Object storage | `kv.sqlite` | `kv.sqlite@registry.local:9000` |
| Vector storage | `vec.sqlite-vec` | `vec.sqlite-vec@registry.local:9000` |
| Runtime | `runtime.container` | `runtime.container@registry.local:9000` |
| Harness | `cli` | `cli@localhost` |

The identity is `<service>.<backend>` — directly derived from the NATS subject segments that already exist. No new identity system. The registry already tracks these capabilities; this just formalizes them as identities.

Git log with full identity:

```
$ git log --oneline --format="%h %an <%ae>: %s"

a1b2c3d cli <cli@localhost>: invoke: cli → support-agent
b3c4d5e support-agent <support-agent@registry.local:9000>: request: support-agent → infer.ollama
c4d5e6f infer.ollama <infer.ollama@registry.local:9000>: response: infer.ollama → support-agent
d5e6f7a support-agent <support-agent@registry.local:9000>: complete: support-agent → cli
```

Every entity that produces a message has a verifiable identity. `git log --author=infer.ollama` shows all inference responses. `git shortlog -sn` ranks every participant — agents, services, harnesses — by activity.

### 3. Session-id propagation to all message types

All five message types carry `session-id` in their NATS headers. The session flows from Invoke through the entire call chain:

- **Invoke**: Already has `session: SessionId` field
- **Complete**: Add `session: SessionId` — echoed from the Invoke that started it
- **Request**: Add `session: SessionId` — the agent's session context flows to services
- **Response**: Add `session: SessionId` — echoed from the Request
- **Delegate**: Already has `session: SessionId` field

The DagCaptureWorker already checks for `session-id` in headers and drops messages without it. With propagation, all five types reach the workers. The git projection becomes complete — every message in the protocol trace appears as a commit.

### 4. Agent invoke protocol change

The container runtime communicates with agents via two HTTP interfaces:

- **Inbound** (runtime → agent): `POST /invoke` on the container's port 8080. The runtime sends user input and reads the agent's response. This is the `/invoke` path in `dispatch_to_container()`.
- **Outbound** (agent → runtime): The HTTP bridge (`HttpBridge`) on a host-assigned port. The agent calls `/kv/get`, `/infer`, `/delegate`, etc. This path is unchanged.

The protocol change affects **inbound only** — the `/invoke` response format.

#### Current protocol

```
POST /invoke HTTP/1.1
X-Vlinder-Session: ses-abc123
Content-Length: 42

<raw payload bytes>
```

Response:

```
HTTP/1.1 200 OK
Content-Length: 128

<raw response bytes — becomes CompleteMessage.payload>
```

The entire response body is treated as `CompleteMessage.payload`. No structure, no metadata. The runtime cannot distinguish agent output from agent logs.

#### New protocol

Request is unchanged. Response becomes structured JSON:

```
HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: 256

{
  "response": "<agent's answer — becomes CompleteMessage.payload>",
  "stderr": "<agent's log stream — becomes ContainerDiagnostics.stderr>"
}
```

| Field | Required | Destination |
|-------|----------|-------------|
| `response` | Yes | `CompleteMessage.payload` |
| `stderr` | No (default: empty) | `ContainerDiagnostics.stderr` |

The runtime parses the JSON, extracts `response` as the payload, and places `stderr` into diagnostics. If parsing fails (e.g., legacy agent returning raw bytes), the runtime falls back to treating the entire body as payload with empty stderr — backwards compatible.

#### Outbound bridge diagnostics

The outbound bridge protocol (agent → `/kv/get`, `/infer`, etc.) is unchanged from the agent's perspective — agents still POST JSON to bridge endpoints and get responses back. But the bridge now captures diagnostics on every call it intercepts.

When the agent calls `/infer`:
1. Bridge records `received_at_ms` and `request_bytes` from the HTTP request
2. Bridge maps the path to an `op`, merges it into the JSON, emits `RequestMessage` with `RequestDiagnostics`
3. Service worker processes the request, emits `ResponseMessage` with `ServiceDiagnostics`
4. Bridge returns the response to the agent

The bridge is the only component that sees both sides of every outbound call. It captures what the agent asked for (RequestDiagnostics) and the platform measures what the service did (ServiceDiagnostics). Together, these two commits in the git log tell the complete story of every service interaction.

#### What doesn't change

- The `/invoke` request format (raw bytes with `X-Vlinder-Session` header)
- The outbound bridge endpoint format (agent POSTs JSON to `/kv/get`, `/infer`, etc.)
- The bridge endpoint mapping (`op_for_path`)
- The `ServiceRouter` dispatch path

The agent SDK templates will be updated to return the new JSON format for `/invoke` responses. Existing agents that return raw bytes continue to work via the fallback. Outbound bridge calls are unchanged from the agent's perspective.

### 5. Implementation: every emitter populates its diagnostics

Every platform component that creates a message is responsible for populating its diagnostics. No message leaves a component without diagnostics — the types enforce this at compile time (the field is not `Option`).

#### Harness → InvokeMessage + InvokeDiagnostics

**File:** `src/domain/harness.rs`

The harness creates `InvokeMessage` in `invoke()`. It already knows the version (`env!("CARGO_PKG_VERSION")`) and the history turn count from the session. Populates `InvokeDiagnostics` at construction time.

#### HTTP Bridge → RequestMessage + RequestDiagnostics

**File:** `src/runtime/http_bridge.rs`

The bridge creates `RequestMessage` via `ServiceRouter::dispatch()` when the agent calls an SDK endpoint. It already has the endpoint path (from the HTTP request line), the body size (from `Content-Length`), and the sequence number. Records `received_at_ms` from `SystemTime::now()` when the request arrives.

#### Service Workers → ResponseMessage + ServiceDiagnostics

Each service worker receives a `RequestMessage`, calls its backend, and emits a `ResponseMessage`. The worker is where the diagnostics are born — it holds the backend connection and measures the call.

| Worker | File | Metrics it captures |
|--------|------|---------------------|
| `InferenceServiceWorker` | `src/domain/workers/inference.rs` | Model name, input/output tokens (from Ollama/OpenRouter response), duration |
| `EmbeddingServiceWorker` | `src/domain/workers/embedding.rs` | Model name, embedding dimensions, duration |
| `ObjectServiceWorker` | `src/domain/workers/object.rs` | Operation (get/put/list/delete), bytes transferred, duration |
| `VectorServiceWorker` | `src/domain/workers/vector.rs` | Operation (store/search/delete), bytes transferred, duration |

All workers already call their backends and return results — duration is a `std::time::Instant::now()` wrapper around the call. Token counts come from the Ollama/OpenRouter API response fields that are currently ignored. Bytes transferred is `payload.len()`.

#### Container Runtime → CompleteMessage + ContainerDiagnostics

**File:** `src/runtime/container.rs`

The runtime creates `CompleteMessage` in the task completion path (after the agent's `/invoke` response returns). It already holds the `container_id` and has access to the agent's `executable` (image ref). Adds:
- `Instant::now()` around `dispatch_to_container()` for duration
- Parses the structured JSON response to extract stderr
- Caches `podman version` output at startup (once per runtime lifecycle)
- Calls `podman inspect <image> --format '{{.Digest}}'` at container start for image digest

#### Container Runtime (Bridge) → DelegateMessage + DelegateDiagnostics

**File:** `src/runtime/http_bridge.rs`

When the agent calls `/delegate`, the bridge intercepts it and creates a `DelegateMessage`. The delegate triggers container execution of the target agent, so `DelegateDiagnostics` wraps `ContainerDiagnostics` with the same data sources as Complete.

### Commit tree with diagnostics

```
tree
├── payload              (message content — raw bytes)
├── stderr               (agent stderr — raw bytes, Complete/Delegate only)
├── diagnostics.toml     (typed diagnostics, serialized)
├── agent.toml           (agent manifest from registry)
├── models/
│   ├── <model-name>.toml
│   └── ...
└── platform.toml        (vlinder version, registry host)
```

`diagnostics.toml` for a Complete message:

```toml
[container]
duration_ms = 2300
stderr_bytes = 142

[container.runtime]
engine_version = "5.3.1"
image_ref = "localhost/support-agent:latest"
image_digest = "sha256:a80c4f17..."
container_id = "abc123def456"
```

`diagnostics.toml` for an inference Response:

```toml
[service]
service = "infer"
backend = "ollama"
duration_ms = 1800

[service.metrics.inference]
tokens_input = 512
tokens_output = 908
model = "phi3:latest"
```

## Consequences

- Diagnostics are **platform-verified** — populated by the emitting platform component, not self-reported by agents or external services
- Every message type carries diagnostics specific to its emitter — strongly typed, not optional fields
- Services have registry-backed identities — `git log --author` works for agents, services, and harnesses
- All five message types propagate session-id — the git projection captures the complete protocol trace
- `git show <commit>:stderr` shows the agent's logs for that exact invocation
- `git show <commit>:diagnostics.toml` shows execution metrics (tokens, latency, container info)
- `git shortlog -sn` ranks every entity in the system by activity
- The agent contract evolves to return structured output (response + stderr), but the agent never sees or controls the diagnostics struct
- The container runtime populates runtime info from Podman metadata it already has — the agent cannot misreport its container version or image digest
- Service workers populate metrics from data they already compute — Ollama doesn't know about our diagnostics model
- No new identity system — service identity is `<service>.<backend>` derived from existing NATS subjects
- Combined with commit signing (ADR 069), every diagnostic claim is cryptographically attributable to the platform component that made it
