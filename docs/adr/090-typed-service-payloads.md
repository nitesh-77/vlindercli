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

## Decision

Deferred.
