# ADR 087: Inference Passthrough Protocol

## Status

Accepted

## Context

ADR 086 decides that inference moves from a platform-defined pinhole (`model, prompt, max_tokens`) to per-provider transparent proxies on the bridge. This ADR defines the wire format: how requests and responses flow through NATS between the bridge proxy and the inference service workers.

The key constraint: message payloads are strongly typed per provider, pinned to a specific API version. Providers version their APIs — OpenRouter tracks OpenAI API v1, Ollama versions its own endpoints. We don't reinvent these type definitions. Rust SDKs already exist for both providers with complete, versioned, serde-compatible types.

## Decision

### Use provider SDK types directly

The NATS payload types are the provider SDK types — not hand-rolled mirrors.

**OpenRouter**: Use [`async-openai`](https://crates.io/crates/async-openai) types. The crate provides granular feature flags (`chat-completion-types`) to import only the types without the HTTP client. OpenRouter is OpenAI API v1 compatible — these types are an exact match. `CreateChatCompletionRequest`, `CreateChatCompletionResponse`, `ChatCompletionRequestMessage`, `ChatCompletionTool`, `CompletionUsage` — all already defined, including streaming chunk types (`CreateChatCompletionStreamResponse`).

**Ollama** (future iteration): Use [`ollama-rs`](https://crates.io/crates/ollama-rs) types. `ChatMessageRequest`, `ChatMessage`, Ollama-specific options — all typed and versioned.

This means:
- No hand-rolled request/response structs in `src/domain/`
- Full API surface covered — tool calling, structured output, vision, everything the SDK already supports
- Streaming SSE chunk assembly already implemented in the crate
- Version tracking is a `Cargo.toml` bump, same as any dependency

### Cargo.toml

```toml
# Types only — no HTTP client dependency
async-openai = { version = "0.30", default-features = false, features = ["chat-completion-types"] }
```

### NATS subject routing

New subjects for passthrough, separate from legacy:

```
vlinder.request.infer.openrouter.chat.{submission_id}
```

The legacy subject `vlinder.request.infer.openrouter.run.{submission_id}` continues to work during migration. Workers subscribe to both. The subject encodes the provider and endpoint — the worker knows what type to deserialize without inspecting the payload.

When Ollama is added later:

```
vlinder.request.infer.ollama.chat.{submission_id}
```

### RequestMessage payload

The `RequestMessage.payload` field carries the serialized `CreateChatCompletionRequest` (from `async-openai`) as JSON bytes. The type is implied by the NATS subject — no discriminator field needed. The existing `RequestMessage` envelope fields (submission, session, agent_id, sequence, diagnostics, state) remain unchanged.

The bridge:
1. Parses the agent's HTTP request body into `CreateChatCompletionRequest`
2. Validates model name against declared models
3. Notes `stream` field for later re-emission
4. Serializes to JSON bytes as `RequestMessage.payload`

### ResponseMessage payload

The `ResponseMessage.payload` field carries the serialized `CreateChatCompletionResponse` as JSON bytes. The worker:
1. Deserializes `CreateChatCompletionRequest` from `RequestMessage.payload`
2. Determines streaming: reads the `stream` field
3. If `stream` is `true`: sends to OpenRouter with streaming, buffers complete SSE stream, assembles into `CreateChatCompletionResponse` (the `async-openai` crate already provides streaming chunk types for this assembly)
4. If `stream` is `false` or absent: sends to OpenRouter, parses response directly
5. Extracts token counts from `CompletionUsage` for `ServiceDiagnostics`
6. Serializes `CreateChatCompletionResponse` as `ResponseMessage.payload`

The bridge:
1. Deserializes `CreateChatCompletionResponse` from `ResponseMessage.payload`
2. If the original request had `stream: true`: converts back to SSE chunks and writes to the agent's HTTP response with `Content-Type: text/event-stream`
3. If non-streaming: serializes as JSON and writes with `Content-Type: application/json`

### Store-and-forward streaming

When the worker requests streaming from OpenRouter, it receives SSE chunks:

```
data: {"id":"...","choices":[{"delta":{"role":"assistant"},...}]}
data: {"id":"...","choices":[{"delta":{"content":"Hello"},...}]}
data: {"id":"...","choices":[{"delta":{"content":" world"},...}]}
data: {"id":"...","choices":[{"delta":{},"finish_reason":"stop",...}],"usage":{...}}
data: [DONE]
```

The `async-openai` crate provides `CreateChatCompletionStreamResponse` for each chunk. The worker assembles these into a single `CreateChatCompletionResponse` by:
1. Concatenating `delta.content` fields across chunks into the final `message.content`
2. Collecting `delta.tool_calls` if present
3. Taking `finish_reason` from the final chunk
4. Taking `usage` from the final chunk (OpenRouter includes usage in the last chunk)
5. Preserving `id`, `model`, `created`, `object` from the first chunk

The assembled response is indistinguishable from a non-streaming response. One type, one NATS message, one DAG node.

Streaming exists for humans who read slower than LLMs generate. Agents are machine consumers — they process the complete response in microseconds regardless of delivery pattern. Store-and-forward eliminates NATS streaming complexity (no multi-message protocol, no ordering concerns, no redelivery interleaving) at zero perceived cost, because LLM generation latency dominates and conceals the buffering.

### Bridge re-streaming

When the bridge re-emits as SSE (original request had `stream: true`), it converts the `CreateChatCompletionResponse` back to SSE chunks using `CreateChatCompletionStreamResponse`:

1. First chunk: `{"choices":[{"delta":{"role":"assistant"},...}]}`
2. Content chunks: split `message.content` into reasonable pieces
3. Tool call chunks: emit `tool_calls` deltas if present
4. Final chunk: `{"choices":[{"delta":{},"finish_reason":"stop",...}],"usage":{...}}`
5. `data: [DONE]`

The exact chunking granularity doesn't matter — the agent receives the same complete content regardless of how it's split. This is cosmetic fidelity for SDKs that expect SSE framing.

### Legacy coexistence

During migration, both protocols coexist:

| Subject pattern | Payload type | Worker |
|---|---|---|
| `vlinder.request.infer.openrouter.run.*` | `InferRequest { model, prompt, max_tokens }` | Legacy worker |
| `vlinder.request.infer.openrouter.chat.*` | `CreateChatCompletionRequest` (async-openai) | Passthrough worker |

The bridge proxy port routes to the `chat` subject. The state machine `Infer` action routes to the legacy `run` subject. Both work simultaneously. After migration is validated, the legacy subject and worker are removed along with `AgentAction::Infer`.

### ServiceDiagnostics

The existing `ServiceDiagnostics` and `ServiceMetrics::Inference` types work unchanged. The worker extracts from the typed response:

```rust
ServiceMetrics::Inference {
    tokens_input: response.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
    tokens_output: response.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
    model: response.model.clone(),
}
```

No parsing, no string searching — just reading fields from a typed struct provided by the SDK crate.

## Consequences

- NATS payloads use provider SDK types directly — no hand-rolled type definitions to maintain
- `async-openai` types (types-only feature flag) cover the full OpenAI API v1 surface including streaming chunks
- API version tracking is a `Cargo.toml` version bump, same as any dependency
- Model validation, stream detection, and diagnostics extraction are field reads on SDK types
- NATS subject naming encodes provider and endpoint — workers self-select without payload inspection
- Legacy and passthrough protocols coexist on separate subjects during migration
- Store-and-forward streaming uses SDK-provided chunk types for assembly and re-emission
- Adding Ollama later means adding the `ollama-rs` dependency, new subjects, new worker — same pattern, different SDK crate
