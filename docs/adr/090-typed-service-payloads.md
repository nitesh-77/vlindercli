# ADR 090: Typed Service Payloads

## Status

Draft

## Context

### The trait is already correct

The `MessageQueue` trait has one `send_*()` method per message type, each taking a typed message struct:

```rust
fn send_invoke(&self, msg: InvokeMessage) -> Result<(), QueueError>;
fn send_request(&self, msg: RequestMessage) -> Result<(), QueueError>;
fn send_response(&self, msg: ResponseMessage) -> Result<(), QueueError>;
fn send_complete(&self, msg: CompleteMessage) -> Result<(), QueueError>;
fn send_delegate(&self, msg: DelegateMessage) -> Result<(), QueueError>;
```

The message type determines which method is called. Routing is already type-driven at the trait level. The trait doesn't need to change.

### The payload is bytely-typed but domain-specific

`RequestMessage` carries `payload: Vec<u8>` alongside a routing triple (`service: ServiceType`, `backend: String`, `operation: Operation`). The payload content is domain-specific — `KvGetRequest`, `InferRequest`, `OpenRouterChatRequest` — but the type system sees `Vec<u8>`. The domain specificity is erased at the message level. Nothing enforces that the payload matches the routing — a `KvGetRequest` can be stuffed into a message routed to `infer.ollama.run`. This compiles and fails at runtime.

### Subject computation is duplicated inside implementations

Both `InMemoryQueue` and `NatsQueue` build subject strings by reading `msg.service`, `msg.backend`, `msg.operation` inside their `send_request()` implementations. Same format string, duplicated across both implementations, for every message type.

### The subject format is already a type hierarchy

NATS subjects encode the routing triple as dot-delimited segments: `vlinder.{sub}.req.{agent}.{service}.{backend}.{operation}.{seq}`. The segments `infer.openrouter.chat` are a type path spelled as strings. The type hierarchy is there — the compiler just can't see it.

### The erasure point is `QueueBridge.send_service_request()`

Every `SdkContract` method follows the same pattern:

```rust
fn infer(&self, model: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
    let req = InferRequest { model, prompt, max_tokens };       // domain-specific type
    let payload = serde_json::to_vec(&req)...;                  // erasure: typed → Vec<u8>
    self.send_service_request(
        ServiceType::Infer, backend, Operation::Run, payload    // routing triple + raw bytes
    )
}
```

The domain-specific type (`InferRequest`, `KvGetRequest`, `EmbedRequest`, etc.) is serialized to `Vec<u8>` via `serde_json::to_vec()`, then passed alongside a manually-specified routing triple to `send_service_request()`. Nine call sites, same pattern. The routing triple and the payload can disagree — nothing connects them.

`send_service_request()` is the funnel: it takes `(ServiceType, String, Operation, Vec<u8>)` and builds a `RequestMessage`. This is where domain specificity is erased.

### The routing triple is redundant with the payload type

Each `SdkContract` method hardcodes a routing triple alongside the payload:

- `kv_get()` → `(Kv, backend, Get)` + `KvGetRequest`
- `infer()` → `(Infer, backend, Run)` + `InferRequest`
- `embed()` → `(Embed, backend, Run)` + `EmbedRequest`

`ServiceType::Infer` and `Operation::Run` are domain enums — they correctly describe the kind of service call. But the domain type `InferRequest` already implies `ServiceType::Infer` and `Operation::Run`. Specifying both is redundant, and the redundancy is where mismatches can happen.

The NATS coupling is not in the enums themselves — those are domain entities. The coupling happens downstream when queue implementations call `msg.service.as_str()` and interpolate into dot-delimited subjects. The domain enums are fine; specifying them independently from the payload type that already implies them is the problem.

### The change is inside the send method

The `MessageQueue::send_*()` trait methods are already correctly shaped — typed message in, transport handles routing. The trait doesn't need to change. Whatever replaces the opaque payload lives inside these existing implementations — not on the trait, not on the message types.

### Two code paths, not a rewrite

The change point is `QueueBridge.send_service_request()`, which accepts `payload: Vec<u8>`. The existing nine call sites continue to use it unchanged. A new type-aware method handles the OpenRouter typed payload — it knows the type, derives routing from it, and serializes it. The two paths coexist:

- **Known type** (OpenRouter chat completion): new type-aware method on `QueueBridge`
- **Everything else**: existing `send_service_request(ServiceType, String, Operation, Vec<u8>)` unchanged

No enum wrapping all cases. No refactoring existing call sites. The typed path is new code alongside old code.

### Protocol and provider are orthogonal

The routing triple has three dimensions: `(service, backend, operation)`. The `backend` dimension identifies a provider — where the request is forwarded (OpenRouter, Ollama, Azure). The payload carries a protocol — the wire shape of the request (OpenAI Chat Completion, Anthropic Messages, AWS Bedrock Invoke). These are independent dimensions. An agent can call an Anthropic model via OpenRouter using the Anthropic Messages protocol.

Most inference providers converge on the OpenAI Chat Completion protocol (`CreateChatCompletionRequest` / `CreateChatCompletionResponse`). The genuinely distinct protocols are few:

- **OpenAI Chat Completion** — OpenRouter, Ollama, Azure, Mistral, Google (de facto standard)
- **Anthropic Messages** — different request/response shape
- **AWS Bedrock** — own envelope format

Provider determines URL, authentication, and error shapes. Protocol determines the request/response structure. The two vary independently.

### The orthogonality extends beyond inference

The same protocol/provider split applies to every service type: object storage (S3, Redis, Azure Blob), vector storage (Pinecone, Qdrant, pgvector), embeddings (OpenAI, Cohere), SQL (Postgres, MySQL). Each has typed libraries with real request/response structs. The `Vec<u8>` payload erases all of them equally. Whatever pattern emerges here informs typed payloads for all service types.

### Worker topology is orthogonal to typing

Once payloads are strongly typed, worker organization is a deployment decision. A worker subscribes to subject filters. One worker per protocol, one per provider, one handling everything — just change the filter. Worker topology doesn't constrain the type system design.

### A provider can serve multiple protocols

A single provider can support multiple wire protocols. OpenRouter serves both OpenAI Chat Completion (`/v1/chat/completions`) and Anthropic Messages (`/v1/messages`) — same URL base, same API key, same error shapes, different request/response structures. The protocol the agent uses determines which endpoints are available inside the container.

### The manifest must declare the protocol

The agent manifest currently declares services and models but not which protocol the agent speaks. This is a missing dimension. An agent that uses OpenRouter with the Anthropic Messages protocol needs to express: I use the **Infer** service, via **OpenRouter** backend, speaking **Anthropic Messages** protocol. The protocol declaration constrains the bridge proxy — it determines which HTTP paths are available inside the container.

### The bridge proxy uses the manifest as its route table

The bridge proxy is the HTTP server that receives SDK requests from inside the container. The HTTP path discriminates the protocol: `/v1/chat/completions` is OpenAI Chat Completion, `/v1/messages` is Anthropic Messages. The bridge proxy registers route handlers only for protocols declared in the manifest. Undeclared protocols are not routed — they 404. The illegal state isn't caught by a runtime validation check; the endpoint doesn't exist.

### Scoping: one provider, two protocols

To reveal the real tensions without overbuilding, the first typed payloads target a single provider (OpenRouter) with two distinct protocols: OpenAI Chat Completion and Anthropic Messages. Same provider — same URL base, same API key, same error shapes — but different request/response wire formats. This holds the provider constant and isolates the protocol dimension. The `Vec<u8>` fallback is preserved for everything else.

## Decision

Deferred.
