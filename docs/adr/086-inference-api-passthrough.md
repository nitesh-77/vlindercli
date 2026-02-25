# ADR 086: Inference API Passthrough

## Status

Accepted

## Context

The current inference endpoint is a pinhole. Agents call `ctx.infer(model, prompt, max_tokens)` — three parameters. The platform strips everything else away: no message arrays, no system prompts, no temperature, no tool calling, no structured output, no stop sequences. The Ollama engine uses `/api/generate` (legacy completion endpoint) instead of `/api/chat`. The OpenRouter engine wraps the single string as `[{role: "user", content: prompt}]`. The response is a plain string — no usage metadata, no finish reason, no tool calls.

This prevents agent authors from using established libraries (LangChain, LiteLLM, OpenAI SDK) that already work well with Ollama and OpenRouter. It also limits what gets captured in the DAG — today we record a prompt string and a response string, not the full conversation history, tool definitions, or structured parameters.

## Decision

### Per-provider transparent proxy on the bridge

The bridge exposes a proxy port for each inference provider. The proxy impersonates the real provider — agents talk to it using standard SDKs pointed at a different URL. The bridge forwards requests to the real provider, captures request and response for DAG recording, and returns the unmodified response.

Starting with OpenRouter only (inference-only, single purpose). Ollama follows as a second iteration, informed by what we learn — Ollama serves both inference and embedding on the same port, which adds complexity.

### Agent manifest configuration

```toml
[inference]
openrouter_port = 2222
```

The container receives the proxy URL as an environment variable:

```
VLINDER_OPENROUTER_URL=http://localhost:2222
```

Agent code uses standard libraries with a custom base URL:

```python
from openai import OpenAI

client = OpenAI(
    base_url=os.environ["VLINDER_OPENROUTER_URL"] + "/api/v1",
    api_key="unused",  # bridge injects credentials
)

response = client.chat.completions.create(
    model="anthropic/claude-sonnet-4",
    messages=[
        {"role": "system", "content": "You are a todo list manager."},
        {"role": "user", "content": user_input},
    ],
    tools=[...],
    temperature=0.3,
)
```

Every major library supports custom endpoints: OpenAI SDK (`base_url`), LangChain (`base_url` on `ChatOpenAI`/`ChatOllama`), LiteLLM (`api_base`). This is how enterprises route through proxies — the capability already exists everywhere.

### Routed through NATS

The proxy does not call the provider directly. It wraps the raw HTTP request body as a `RequestMessage` payload and sends it through NATS, same as today. The inference service worker receives it, forwards to the provider, and sends the response back as a `ResponseMessage`.

This preserves the entire recording infrastructure: transactional outbox (ADR 080), DAG recording, `ServiceDiagnostics`, queue-based routing. NATS latency (~1-2ms) is noise on inference calls that take 500ms-30s.

### Streaming: store-and-forward

Agents may send `stream: true` in their requests. The inference worker sends the request to the provider with streaming enabled, buffers the complete SSE stream, then sends the assembled response as one atomic NATS `ResponseMessage`. The bridge re-emits the response as SSE chunks to the agent.

This is the correct design for machine consumers. Streaming exists for humans who read slower than LLMs generate — agents process the complete response in microseconds regardless of delivery pattern. Store-and-forward eliminates all NATS streaming complexity (no protocol change, no ordering concerns, no redelivery interleaving) at zero perceived cost, because LLM generation latency dominates and conceals the buffering entirely.

### Model validation

The bridge validates that agents only call models declared in `agent.toml`. Model identity is a first-class platform concept — not just for routing, but for future capabilities: A/B testing different models via time travel, accuracy and performance analytics across models, cost tracking per model. The DAG captures the full request (including model name) but the platform must understand model identity at a domain level, not just as opaque passthrough metadata.

### What the bridge does on each request

1. Receive raw HTTP request from agent (any path the provider supports)
2. Extract model name from request body — validate against declared models
3. Wrap request body as `RequestMessage` payload, send through NATS
4. Inference worker receives, forwards to real provider (injecting auth headers)
5. Worker buffers complete response (including SSE streams)
6. Worker extracts token counts for `ServiceDiagnostics`
7. Worker sends complete response as one `ResponseMessage` through NATS
8. Bridge receives `ResponseMessage`, returns response body to agent
9. If agent requested streaming, bridge re-emits as SSE chunks

### What gets captured in the DAG

Full request body: messages array, tool definitions, temperature, all parameters. Full response body: choices, tool calls, usage, finish reason. This is a significant improvement over today's "prompt string in, text string out" — `vlinder timeline log` shows exactly what the LLM saw and did.

### Migration path

1. Build the proxy alongside the existing state machine `Infer` action
2. Migrate todoapp to use `ChatOpenAI(base_url=VLINDER_OPENROUTER_URL)`
3. Validate end-to-end: agent runs, DAG captures full request/response
4. Remove `AgentAction::Infer`, `AgentEvent::Infer`, `SdkContract::infer()`, `InferRequest`

Then repeat the same pattern for `/embed`, then for every other service endpoint, one at a time.

### TLS

Not a concern for this change. Agent-to-bridge is localhost (same trust boundary as existing `/handle` endpoint). Bridge-to-provider is already HTTPS via `ureq`. TLS for non-localhost deployments is a deployment-level decision that applies uniformly to all bridge communication — a separate ADR when bridge-as-process (TODO.md) happens.

## Consequences

- Agent authors use standard SDKs (OpenAI, LangChain, LiteLLM) with no Vlinder-specific learning curve
- Full provider API surface is available: tools, structured output, system prompts, multi-turn, temperature, stop sequences — everything
- DAG captures the complete request/response, making time travel significantly more useful
- Model validation at the bridge enables future A/B testing and analytics capabilities
- Existing NATS infrastructure, recording, and diagnostics are preserved unchanged
- Streaming is supported via store-and-forward — correct for machine consumers, no protocol complexity
- The `InferenceEngine` trait simplifies to raw HTTP forwarding — engines no longer construct requests from individual fields
- Pattern established for migrating remaining service endpoints (embed, kv, vector) to the same passthrough model
