# ADR 090: Typed Service Payloads

**Status:** Draft

## Context

Service messages carry `payload: Vec<u8>` alongside a routing triple. The payload is a domain-specific struct serialized to bytes — type information is erased at the message boundary. Nothing connects the payload to its routing. A `KvGetRequest` can ride a message routed to `infer.openrouter.run`. This compiles and fails at runtime.

The platform's own request types (`InferRequest { model, prompt, max_tokens }`) are lowest-common-denominator abstractions that erase what each provider actually supports. The real payload types already exist in provider SDKs.

## Decision

### Header and payload

Messages split into header (control plane) and payload (data plane). Header carries invocation identity, sequencing, state cursor, diagnostics — same shape for every service call. Payload carries the actual service request/response. Routing derives from the payload, not from separate fields that can disagree.

### Per-provider worker crates with real SDK types

Each provider is its own crate. The crate boundary is the billing relationship — who the customer pays their bill to. Each crate owns its typed payloads, its service handler, and its SDK dependency:

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

### Per-domain error enums plus platform error

Data plane errors use real SDK error types per provider. Control plane errors (queue timeout, missing backend, state conflict) use a single `PlatformError` in `vlinder-core`. This replaces the `[error]` byte prefix convention.

### Transparent proxy

The sidecar is invisible. Agents use standard provider SDKs pointed at `localhost:9000` and get back the exact same response shape, status codes, and error bodies they'd get talking to the provider directly. The platform adds value in the header (time-travel, branching, state snapshots), not the payload.

### SDK types are real dependencies

Provider SDK crates are pinned per-worker-crate and versioned independently. Payloads are deserialized, validated, and recorded in the DAG as structured data. No opaque byte passthrough.

### Version isolation via crate boundaries

Each worker crate has its own `Cargo.toml` pinning its SDK version. Bumping `async-openai` in `vlinder-infer-openrouter` doesn't recompile or affect `vlinder-kv-sqlite`. Cargo handles semver-incompatible transitive deps coexisting in the same binary. The per-provider crate boundary is the version firewall.

### Sidecar is the versioned boundary

The sidecar links worker crates as optional dependencies and presents typed API surfaces to the agent container. Free tier ships all worker crates at latest. Custom sidecar builds select which worker crates (and at which versions) to link — this is just editing a `Cargo.toml`.

### Migration is per-endpoint

One sidecar URL at a time. Add `/v1/chat/completions` alongside `/services/infer`. Both coexist. When the last legacy endpoint migrates, delete `Legacy`, `legacy_bytes()`, `Vec<u8>` payloads, and `check_worker_error()`.

## Consequences

- `RequestPayload::Legacy(Vec<u8>)`, `legacy_bytes()`, `check_worker_error()`, `[error]` prefix convention — all deleted
- `vlinder-payloads` crate deleted — each worker crate owns its types
- Custom types like `InferRequest { model, prompt, max_tokens }` replaced by SDK types
- Agent SDKs become standard provider libraries — no Vlinder-specific code
- Adding a provider = one new crate with its SDK dependency
- Provider SDK version bumps are per-crate releases, not platform-wide
