# ADR 090: Typed Service Payloads

**Status:** Accepted

## Context

Service messages carry `payload: Vec<u8>` alongside a routing triple. The payload is a domain-specific struct serialized to bytes — type information is erased at the message boundary. Nothing connects the payload to its routing. A `KvGetRequest` can ride a message routed to `infer.openrouter.run`. This compiles and fails at runtime.

The platform's own request types (`InferRequest { model, prompt, max_tokens }`) are lowest-common-denominator abstractions that erase what each provider actually supports. The real payload types already exist in provider SDKs.

## Decision

### Header and payload

Messages split into header (control plane) and payload (data plane). Header carries invocation identity, sequencing, state cursor, diagnostics — same shape for every service call. Payload carries the actual service request/response. Routing derives from the payload, not from separate fields that can disagree.

### Per-domain payload enums with real SDK types

One enum per service domain. Variants use types from provider SDK crates, not custom Vlinder structs:

```rust
enum InferPayload { OpenAi(CreateChatCompletionRequest), Anthropic(CreateMessageRequest) }
enum KvPayload { Redis(/* redis SDK types */) }
enum VectorPayload { Qdrant(/* qdrant SDK types */) }
```

### Per-domain error enums plus platform error

Data plane errors use real SDK error types per provider. Control plane errors (queue timeout, missing backend, state conflict) use a single `PlatformError`. This replaces the `[error]` byte prefix convention.

### Transparent proxy

The sidecar is invisible. Agents use standard provider SDKs pointed at `localhost:9000` and get back the exact same response shape, status codes, and error bodies they'd get talking to the provider directly. The platform adds value in the header (time-travel, branching, state snapshots), not the payload.

### SDK types are real dependencies

Provider SDK crates are pinned in `Cargo.toml` and versioned with the platform release. Payloads are deserialized, validated, and recorded in the DAG as structured data. No opaque byte passthrough.

### Contracts live in `vlinder-payloads`

New crate shared between sidecar and workers. Per-domain modules, Cargo features per provider (`openai`, `anthropic`, `redis`, `qdrant`). Core doesn't touch it. `vlinder-proto` stays for gRPC.

### Sidecar is the versioned boundary

The sidecar's HTTP endpoints define the contract with the agent container. Core is internal — sidecar and workers rebuild together. Cargo features map to a provider menu: free tier ships `--all-features` latest, paid tier builds custom sidecar images with a chosen subset.

### Migration is per-endpoint

One sidecar URL at a time. Add `/v1/chat/completions` alongside `/services/infer`. Both coexist. When the last legacy endpoint migrates, delete `Legacy`, `legacy_bytes()`, `Vec<u8>` payloads, and `check_worker_error()`.

## Consequences

- `RequestPayload::Legacy(Vec<u8>)`, `legacy_bytes()`, `check_worker_error()`, `[error]` prefix convention — all deleted
- Custom types like `InferRequest { model, prompt, max_tokens }` replaced by SDK types
- Agent SDKs become standard provider libraries — no Vlinder-specific code
- Adding a provider = one enum variant + one feature flag
- Provider SDK version bumps are platform releases
