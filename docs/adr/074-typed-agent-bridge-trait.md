# ADR 074: Typed Agent Bridge Trait

## Status: Draft

## Context

The platform exposes services to agent containers (KV, vector, infer, embed, delegate, wait). Today this contract is stringly-typed:

1. The container sends JSON with an `op` string field
2. `SdkMessage` deserializes it (runtime validation only)
3. `hop()` matches the variant to resolve service routing
4. `ServiceRouter::dispatch` matches again for delegate/wait vs everything else
5. Raw bytes are forwarded to service workers who deserialize a third time

The typed `SdkMessage` enum exists solely to validate JSON — nobody calls its variants from Rust. The actual contract is `dispatch(Vec<u8>) -> Result<Vec<u8>>`, a bag of bytes in, bag of bytes out.

This makes the bridge untestable without HTTP, impossible to swap transports, and invisible at compile time.

## Decision

Replace `ServiceRouter::dispatch(Vec<u8>)` with a typed `AgentBridge` trait:

```rust
pub(crate) trait AgentBridge: Send + Sync {
    fn kv_get(&self, path: &str, state: Option<&str>) -> Result<Vec<u8>, BridgeError>;
    fn kv_put(&self, path: &str, content: &str, state: Option<&str>) -> Result<KvPutResult, BridgeError>;
    fn kv_list(&self, path: &str, state: Option<&str>) -> Result<Vec<u8>, BridgeError>;
    fn kv_delete(&self, path: &str, state: Option<&str>) -> Result<Vec<u8>, BridgeError>;
    fn vector_store(&self, key: &str, vector: &[f32], metadata: &str) -> Result<Vec<u8>, BridgeError>;
    fn vector_search(&self, vector: &[f32], limit: u32) -> Result<Vec<u8>, BridgeError>;
    fn vector_delete(&self, key: &str) -> Result<Vec<u8>, BridgeError>;
    fn infer(&self, model: &str, prompt: &str, max_tokens: u32) -> Result<Vec<u8>, BridgeError>;
    fn embed(&self, model: &str, text: &str) -> Result<Vec<u8>, BridgeError>;
    fn delegate(&self, agent: &str, input: &str) -> Result<DelegateHandle, BridgeError>;
    fn wait(&self, handle: &DelegateHandle) -> Result<Vec<u8>, BridgeError>;
}
```

State (ADR 055) is explicit in the KV signatures — state in, state out. `KvPutResult` includes the new state hash. Each call is pure; no hidden mutation. The HTTP transport threads state between successive container calls (bookkeeping), but the trait itself is stateless.

## Consequences

- **HTTP becomes one transport.** `HttpBridge` parses JSON at the boundary, calls typed methods. A gRPC transport or a direct in-process bridge would implement the same trait.
- **`SdkMessage::hop()` disappears.** Each method knows its service and backend — routing is static per method, not dynamic per match arm.
- **`SdkMessage` stays** but becomes HTTP-layer concern only — it validates inbound JSON and maps to trait method calls. It no longer flows into the service layer.
- **Testable without HTTP.** Mock `AgentBridge` to test scheduling. Test bridge implementation with a mock queue. No TCP sockets needed.
- **One module.** The trait + queue-backed implementation + HTTP transport live in a single `bridge` module, consolidating `http_bridge.rs`, `service_router.rs`, and `dispatch_to_container`.
