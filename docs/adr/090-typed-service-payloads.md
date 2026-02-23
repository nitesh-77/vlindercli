# ADR 090: Typed Service Payloads

**Status:** Draft

## Context

Service messages carry `payload: Vec<u8>` alongside a routing triple (`service`, `backend`, `operation`). The payload is a domain-specific struct serialized to bytes — type information is erased at the message boundary. Nothing connects the payload to its routing. A `KvGetRequest` can ride a message routed to `infer.openrouter.run`. This compiles and fails at runtime.

The routing triple exists because the payload is opaque. External metadata tells the system where to send bytes it can't inspect. The platform's own request types (`InferRequest { model, prompt, max_tokens }`) are lowest-common-denominator abstractions that erase what each provider actually supports. The real payload types already exist in provider SDKs.

`RequestMessage` bundles control plane concerns (identity, sequencing, state) with data plane concerns (what service, what operation, what payload). These are separate concerns forced into one struct.

### Call chain comparison: agent calls inference

**Current (bytes + LCD types)**

```
Agent POST http://localhost:9000/services/infer
  body: {"model":"…","prompt":"hello","max_tokens":256}      ← Vlinder LCD type
│
├ http_server::handle_infer
│   deserialize → sidecar's own InferRequest{model,prompt,max_tokens}
│   call bridge.infer(model, prompt, max_tokens)              ← 3 loose strings
│
├ QueueBridge::infer
│   resolve backend via registry
│   build vlinder-core InferRequest, serialize → Vec<u8>      ← type erased to bytes
│   send_service_request(ServiceBackend::Infer(b), Op::Run, bytes)
│                                                              ← routing manually specified
├ QueueBridge::send_service_request
│   build RequestMessage::new(…all fields…)                    ← control+data+routing bundled
│   queue.call_service(request)
│
├ NatsQueue::send_request
│   subject: vlinder.{t}.{s}.req.{agent}.infer.openrouter.run.1
│   headers: msg-id, …, service, backend, operation            ← routing in headers AND subject
│   body: payload.legacy_bytes()                               ← opaque bytes
│
│ ── NATS wire ──
│
├ InferenceServiceWorker::tick
│   receive_request(ServiceType::Infer, "openrouter", Op::Run)
│   deserialize InferRequest from legacy_bytes()               ← could fail at runtime
│
├ InferenceServiceWorker::handle_infer
│   engine.infer(&req.prompt, req.max_tokens)                  ← LCD trait
│
├ OpenRouterEngine::infer                                      ← inside the engine
│   build CreateChatCompletionRequest from prompt+max_tokens   ← reconstruct real type from LCD
│   ureq::post(url).send_json(&request)
│   ← CreateChatCompletionResponse
│   extract text + 2 integers → InferenceResult                ← discard everything else
│
├ ResponseMessage{payload: text.into_bytes()}                   ← response is text bytes
│ ── NATS wire ──
│
├ QueueBridge → response.payload.legacy_bytes() → String
├ http_server → Ok(text.into_bytes())
│
Agent receives: plain text
  no usage, no finish_reason, no choices
  8+ type transitions, several lossy
```

**New (typed end-to-end)**

```
Sidecar main loop:
  invoke = queue.receive_invoke()
  server = start_provider_server(invoke.clone())       ← spawned per invoke
  post_to_agent(invoke.payload)                        ← triggers agent
│
Agent: client = OpenAI(base_url="http://openrouter.vlinder.local/v1")
       client.chat.completions.create(model="…", messages=[…])
                                                               ← real SDK call, no Vlinder types
│
├ DNS: openrouter.vlinder.local → 127.0.0.1:80
├ Provider server dispatches by Host header → vlinder-infer-openrouter routes
│
├ Route handler
│   serde_json::from_slice::<CreateChatCompletionRequest>(body)
│                                                    ── HTTP byte boundary (1 of 2) ──
│
├ Build ServiceHeader from owned InvokeMessage
│   {id, timeline, submission, session, agent_id, sequence, state, diagnostics}
│
├ Derive routing from ProviderRoute:
│   route.service_backend → ServiceBackend::Infer(OpenRouter)
│   route.operation → Operation::Run                            ← no manual routing
│
├ Queue builds RoutingKey, routing_key_to_subject()
│   subject: vlinder.{t}.{s}.req.{agent}.infer.openrouter.run.1
│   headers: ServiceHeader fields only (control plane)
│   body: serde_json::to_vec(&request)
│                                                    ── NATS byte boundary (2 of 2) ──
│
│ ── NATS wire ──
│
├ Worker receives
│   serde_json::from_slice::<CreateChatCompletionRequest>(&body)
│   headers → ServiceHeader                                    ← type known from subject pattern
│
├ OpenRouterEngine::chat_completion(&request)
│   ureq::post(url).send_json(&request)                        ← same type, no conversion
│   ← CreateChatCompletionResponse                             ← same type, no extraction
│
├ ServiceHeader + CreateChatCompletionResponse → queue
│ ── NATS wire ──
│
├ Provider server receives typed response
├ Route handler: serde_json::to_vec(&response)
│                                                    ── HTTP byte boundary ──
│
Agent receives: CreateChatCompletionResponse
  usage, choices, finish_reason — everything
  identical to calling OpenRouter directly
  2 byte boundaries, 0 lossy conversions

Sidecar main loop:
  agent responds → server drops                        ← provider server lifecycle ends
```

## Decision

### Types flow end-to-end

Typed payloads flow from the HTTP boundary through to the engine. The only byte serialization happens at infrastructure boundaries: the HTTP network edge and inside the NATS queue implementation. Everything between is typed. It is impossible to represent an illegal state — you can't send a `KvGetRequest` to an inference queue because it won't compile.

```
HTTP body (bytes)           ── network boundary ──
  → CreateChatCompletionRequest   ── typed from here ──
  → ServiceHeader + typed payload
  → MessageQueue trait
  ───── NATS serializes (infra boundary) ─────
  ───── NATS deserializes ─────
  → ServiceHeader + typed payload
  → Worker
  → OpenRouterEngine::chat_completion()
  → CreateChatCompletionResponse  ── typed all the way back ──
HTTP body (bytes)           ── network boundary ──
```

### RequestMessage is replaced by ServiceHeader

`RequestMessage` collapses. The routing triple (`service`, `backend`, `operation`) is redundant when the payload type IS the routing. What remains is the header — control plane fields that are the same for every service call:

```rust
struct ServiceHeader {
    id: MessageId,
    timeline: TimelineId,
    submission: SubmissionId,
    session: SessionId,
    agent_id: AgentId,
    sequence: Sequence,
    state: Option<String>,
    diagnostics: RequestDiagnostics,
}
```

A typed service call is `(ServiceHeader, T)` where `T` is the real SDK type. Same applies to responses. The DAG layer gets human-readable labels ("infer", "openrouter") from a trait on the payload type, not from separate routing fields.

### Per-provider worker crates

Each provider is its own crate. The crate boundary is the billing relationship — who the customer pays their bill to. Each crate owns its typed payloads, its routes, its service handler, and its SDK dependency:

```
vlinder-infer-openrouter    # async-openai (OpenRouter's protocol today)
vlinder-infer-openai        # async-openai (OpenAI direct)
vlinder-infer-anthropic     # anthropic SDK
vlinder-infer-ollama        # ollama API (local)

vlinder-kv-sqlite           # rusqlite (local)
vlinder-kv-dynamodb         # aws-sdk-dynamodb
vlinder-kv-postgres         # tokio-postgres

vlinder-vector-sqlite-vec   # sqlite-vec (local)
vlinder-vector-qdrant       # qdrant-client
vlinder-vector-pinecone     # pinecone SDK
```

Two crates may use the same SDK internally (openrouter and openai both use `async-openai`). That's a coincidence, not a coupling. If a provider changes their API surface, exactly one crate changes.

### Provider server is scoped to one invoke

The provider server is not a long-lived background service. It is spawned per invoke and dies when the agent responds. Its lifetime is the same as `handle_invoke()`.

```rust
fn handle_invoke(&self, invoke: &InvokeMessage) {
    let server = start_provider_server(invoke.clone());
    self.post_to_agent(&invoke.payload);
    self.wait_for_response();
    // server drops, port freed
}
```

The provider server owns its `InvokeMessage` — no shared state, no locks, no channels. It also owns its own `SequenceCounter`, created fresh per invoke.

### Provider server is self-contained

The only input is the `InvokeMessage`. Everything else is derived:

1. **Agent requirements** — from `invoke.agent_id`, look up in registry (connected from config)
2. **Which providers** — from agent's `requirements.services`
3. **Host table** — call provider crate functions (`vlinder_infer_openrouter::provider_host()`)
4. **Queue connection** — from config (env vars)
5. **Routing** — from `ProviderRoute.service_backend` + `ProviderRoute.operation`
6. **Invoke context** — the owned `InvokeMessage`

The sidecar does not pass hosts, queue connections, or routing info. The provider server constructs what it needs.

### Sidecar becomes a lifecycle manager

The sidecar shrinks to a loop that receives invokes from NATS, spawns a provider server, triggers the agent, and waits for the response:

```
loop {
    invoke = queue.receive_invoke()
    server = start_provider_server(invoke.clone())
    post_to_agent(invoke.payload)
    wait_for_response()
    send_complete()
    // server drops
}
```

`QueueBridge`, `http_server` (port 9000), LCD types, `[error]` prefix, manual routing — all dead. The provider server replaces them.

### Each crate owns a hostname

Inside the agent container, each provider crate serves its own virtual host on port 80:

```
openrouter.vlinder.local    → vlinder-infer-openrouter routes
openai.vlinder.local        → vlinder-infer-openai routes
qdrant.vlinder.local        → vlinder-vector-qdrant routes
```

The agent uses standard provider SDKs pointed at the `.vlinder.local` hostname. The agent code is:

```python
client = OpenAI(base_url="http://openrouter.vlinder.local/v1")
```

Instead of `https://openrouter.ai/api/v1`. Same SDK, same routes, different hostname. The agent doesn't know Vlinder exists.

Each crate exports its hostname and typed route handlers. The sidecar dispatches by `Host` header. Container DNS maps `*.vlinder.local` to `127.0.0.1`.

### Routing infrastructure stays, becomes derived

`ServiceBackend`, `Operation`, `RoutingKey`, and `routing_key_to_subject()` already exist in `vlinder-core`. `ServiceBackend` is already "type is routing" for the service+backend dimensions — `ServiceBackend::Kv(InferenceBackendType::Ollama)` won't compile. The NATS queue uses `RoutingKey` to build subject strings like `vlinder.{timeline}.{submission}.req.{agent}.infer.openrouter.run.1`. This infrastructure stays.

What changes: these routing dimensions are **derived from the payload type**, not manually specified. Each typed payload implements a trait that produces its `ServiceBackend` and `Operation`:

```rust
trait ServicePayload {
    fn service_backend(&self) -> ServiceBackend;
    fn operation(&self) -> Operation;
}
```

The programmer never writes routing fields. A `CreateChatCompletionRequest` produces `ServiceBackend::Infer(OpenRouter)` + `Operation::Run`. The queue uses these to build the `RoutingKey` and NATS subject. Mismatches between payload and routing are eliminated — the type is the single source of truth, and the routing dimensions are a derivation of it.

### NATS wire format

A NATS message has three parts: subject, headers, and body. The typed payload design maps cleanly onto this:

```
Subject:  vlinder.{timeline}.{submission}.req.{agent}.infer.openrouter.run.1
          └── derived from ServicePayload trait on the typed payload

Headers:  msg-id, timeline-id, submission-id, session-id, agent-id,
          sequence, state, diagnostics
          └── ServiceHeader fields (control plane only)

Body:     serde_json::to_vec(&CreateChatCompletionRequest { ... })
          └── the typed payload, serialized at the NATS boundary
```

The subject is built by `routing_key_to_subject()` from the `ServiceBackend` + `Operation` that the payload type derives. Headers carry `ServiceHeader` — control plane fields identical for every service call. No `service`, `backend`, or `operation` headers — the subject already encodes routing.

The body is the SDK type serialized to bytes. This is one of the two byte boundaries (the other being the HTTP edge). On send: `serde_json::to_vec(&payload)`. On receive: `serde_json::from_slice::<T>(&body)`. The type `T` is known because the worker subscribes to a specific subject pattern — listening on `*.*.*.req.*.infer.openrouter.run.*` means the body is a `CreateChatCompletionRequest`.

### Per-domain errors plus platform error

Data plane errors use real SDK error types per provider, returned in the provider's native error format (e.g., OpenAI's `{"error":{"message":"...","type":"..."}}`). Control plane errors (queue timeout, missing backend, state conflict) use a single `PlatformError` in `vlinder-core`. The `[error]` byte prefix convention is deleted.

### Version isolation via crate boundaries

Each worker crate has its own `Cargo.toml` pinning its SDK version. Bumping `async-openai` in `vlinder-infer-openrouter` doesn't recompile or affect `vlinder-kv-sqlite`. Cargo handles semver-incompatible transitive deps coexisting in the same binary. The per-provider crate boundary is the version firewall.

### Sidecar composition

The sidecar binary links provider crates. The provider server discovers which ones the agent needs and serves their hostnames on port 80 for the duration of each invoke. Free tier ships all provider crates at latest. Custom sidecar builds select which provider crates (and at which versions) to link — this is editing a `Cargo.toml`.

### Migration is per-endpoint

One hostname at a time. Add `openrouter.vlinder.local` alongside the legacy `/services/infer`. Both coexist. When the last legacy endpoint migrates, delete `RequestMessage`, `ResponseMessage`, `RequestPayload::Legacy`, `legacy_bytes()`, `Vec<u8>` payloads, `check_worker_error()`, the routing triple, and the LCD request types.

## Consequences

- `QueueBridge` deleted — provider server handles service calls directly
- `http_server` (port 9000) deleted — agent uses provider hostnames, not `/services/infer`
- `RequestMessage` and `ResponseMessage` replaced by `ServiceHeader` + typed payload
- Routing triple (`service`, `backend`, `operation`) derived from `ProviderRoute` — never manually specified
- `ServiceBackend`, `RoutingKey`, `routing_key_to_subject()` stay — queue infrastructure needs them
- `RequestPayload::Legacy(Vec<u8>)`, `legacy_bytes()`, `check_worker_error()`, `[error]` prefix — all deleted
- Custom types like `InferRequest { model, prompt, max_tokens }` replaced by SDK types
- `vlinder-payloads` crate deleted — each worker crate owns its types
- Agent code uses standard provider SDKs — no Vlinder-specific code, no vendor lock-in
- Adding a provider = one new crate with its hostname, routes, and SDK dependency
- Provider SDK version bumps are per-crate releases, not platform-wide
- Byte serialization pushed to infrastructure boundaries (HTTP edge, NATS wire)
- Provider server scoped to invoke lifetime — no shared mutable state
- Sidecar shrinks to a lifecycle manager (receive invoke, spawn server, trigger agent, wait)
