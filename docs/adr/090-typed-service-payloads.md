# ADR 090: Typed Service Payloads

**Status:** Accepted

## Context

Service messages carried `payload: Vec<u8>` alongside a routing triple (`service`, `backend`, `operation`). Type information was erased at the message boundary — a `KvGetRequest` could ride a message routed to `infer.openrouter.run` and only fail at runtime.

The platform's own request types (`InferRequest { model, prompt, max_tokens }`) were lowest-common-denominator abstractions that erased what each provider actually supports. The real payload types already existed in provider SDKs.

`RequestMessage` bundled control plane concerns (identity, sequencing, state) with data plane concerns (service, operation, payload).

## Decision

### Types flow end-to-end

Typed payloads flow from the HTTP boundary through to the backend. Byte serialization only happens at infrastructure boundaries: the HTTP network edge and inside the NATS queue. Everything between is typed.

```
HTTP body (bytes)               ── network boundary ──
  → typed request (SDK type)    ── typed from here ──
  → ProviderRoute → ServiceBackend + Operation
  → MessageQueue
  ───── NATS wire (bytes) ─────
  → Worker → backend call
  → typed response (SDK type)   ── typed all the way back ──
HTTP body (bytes)               ── network boundary ──
```

### Per-provider crates

Each provider is its own crate. The crate boundary is the billing relationship. Each crate owns its hostname, routes, typed payloads, and SDK dependency:

```
vlinder-ollama                 # inference + embedding (local)
vlinder-infer-openrouter       # inference (OpenRouter)
vlinder-sqlite                 # KV storage (local)
vlinder-sqlite-vec             # vector storage (local)
```

Two crates may use the same SDK internally. That's a coincidence, not a coupling. If a provider changes their API, exactly one crate changes.

### Each crate owns a hostname

Inside the agent container, each provider crate serves a virtual host on port 80:

```
ollama.vlinder.local        → vlinder-ollama routes
openrouter.vlinder.local    → vlinder-infer-openrouter routes
sqlite.vlinder.local        → vlinder-sqlite routes
sqlite-vec.vlinder.local    → vlinder-sqlite-vec routes
```

For stateless services (inference, embedding), the agent uses standard provider SDKs pointed at the `.vlinder.local` hostname:

```python
client = OpenAI(base_url="http://openrouter.vlinder.local/v1")
```

Same SDK, same routes, different hostname.

For stateful services (KV, vector), the API surface belongs to the platform because time travel requires it (ADR 100). The hostname names the backend; the contract is the platform's.

### Provider server is scoped to one invoke

Spawned per invoke, dies when the agent responds. The only input is the `InvokeMessage`. Everything else is derived: agent requirements from registry, provider hosts from crate functions, queue from config.

### Routing is derived from ProviderRoute

Each `ProviderRoute` declares its `ServiceBackend` and `Operation`. The provider server reads these from the matched route. A route's type constraints make it impossible to send a request to the wrong backend.

### Capability-scoped routes

`provider_host()` accepts capability flags so the sidecar only exposes routes the agent actually declared. An embed-only agent gets `/api/embed` but not the three inference routes.

### Per-domain errors

Data plane errors use the provider's native error format. Control plane errors (queue timeout, missing backend) use `PlatformError`. The `[error]` byte prefix convention is deleted.

### Version isolation

Each crate pins its own SDK version. Bumping a dependency in one provider crate doesn't affect others.

### Migration is per-endpoint

One hostname at a time. New provider hostnames coexist with legacy `/services/*` endpoints. When the last legacy endpoint migrates, delete `QueueBridge`, `http_server` (port 9000), LCD types, `legacy_bytes()`, and the `[error]` prefix.

## Status of migration

| Service | Provider | Hostname | Status |
|---------|----------|----------|--------|
| Inference | Ollama | `ollama.vlinder.local` | Done |
| Inference | OpenRouter | `openrouter.vlinder.local` | Done |
| Embedding | Ollama | `ollama.vlinder.local` | Done |
| KV | SQLite | `sqlite.vlinder.local` | Pending |
| Vector | sqlite-vec | `sqlite-vec.vlinder.local` | Pending |

Legacy endpoints removed: `/services/infer`, `/services/embed`.
Legacy endpoints remaining: KV and vector operations in `QueueBridge`.

## Consequences

- Agent code uses standard provider SDKs — no Vlinder-specific types
- Adding a provider = one new crate with hostname, routes, SDK dep
- Byte serialization pushed to infrastructure boundaries only
- Provider server scoped to invoke lifetime — no shared mutable state
- Sidecar shrinks toward a lifecycle manager as migration completes
- Storage API surface is the platform's, not the backend's (ADR 100)
