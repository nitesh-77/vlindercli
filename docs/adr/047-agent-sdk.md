# ADR 047: Agent SDK

## Status

Accepted (validated in 5591eb0)

## Context

ADR 046 establishes that agents are OCI containers executed by Podman. This frees agents from WASM — any language works. But "any language" is a curse without guidance. If agent developers must understand the message protocol (ADR 044), queue subjects, JSON wire format, and container plumbing just to say hello, we've traded one complexity (extism PDK) for a worse one.

### Current agent contract (extism)

Pensieve uses the extism PDK:

```rust
#[host_fn] extern "ExtismHost" {
    fn send(payload: Vec<u8>) -> Vec<u8>;
}

fn call_service(op: &str, payload: impl Serialize) -> Result<String, Error> {
    let mut json = serde_json::to_value(&payload)?;
    json.as_object_mut().unwrap().insert("op".into(), op.into());
    unsafe { send(serde_json::to_vec(&json)?) }
}
```

This is tightly coupled to extism (`#[host_fn]`, `unsafe`, `Vec<u8>` memory model) but the underlying idea is right: the agent calls `send` with an `op` field and gets a response. That's the entire contract.

### What agent developers actually need

1. **Receive input** — the user's submission payload
2. **Call services** — infer, embed, kv, vector
3. **Return output** — the result

Everything else — queue subjects, message types, bridge routing, container lifecycle — is platform concern, not agent concern.

## Decision

### 1. Language SDKs hide the transport

Each SDK provides a typed, idiomatic API for the agent's language. The SDK handles I/O and transport internally. Agent developers never see JSON wire format, HTTP calls, or stdin/stdout.

**Python:**
```python
from vlinder import Agent

agent = Agent()

@agent.process
def handle(payload, ctx):
    response = ctx.infer(model="llama3", prompt=payload)
    ctx.kv.put("last_query", payload)
    results = ctx.vector.search(embedding, limit=5)
    return response
```

**Node:**
```javascript
const { Agent } = require('@vlinder/sdk');
const agent = new Agent();

agent.process(async (payload, ctx) => {
    const response = await ctx.infer({ model: 'llama3', prompt: payload });
    await ctx.kv.put('last_query', payload);
    return response;
});
```

**Go:**
```go
func main() {
    vlinder.Process(func(payload []byte, ctx *vlinder.Context) ([]byte, error) {
        resp, _ := ctx.Infer("llama3", string(payload))
        ctx.KV.Put("last_query", payload)
        return []byte(resp), nil
    })
}
```

### 2. SDK surface area

The SDK exposes five service domains, mapping directly to SdkMessage operations:

| Domain | Methods | Maps to |
|--------|---------|---------|
| `ctx.infer` | `infer(model, prompt)` | `op: "infer"` |
| `ctx.embed` | `embed(model, text)` | `op: "embed"` |
| `ctx.kv` | `put`, `get`, `list`, `delete` | `op: "kv-put"`, `"kv-get"`, etc. |
| `ctx.vector` | `store`, `search` | `op: "vector-store"`, `"vector-search"` |

Each method serializes to SdkMessage JSON, calls the bridge, deserializes the response. The agent sees typed methods; the wire sees `{"op": "infer", "model": "llama3", "prompt": "..."}`.

### 3. Transport: stdin/stdout + HTTP bridge

The SDK uses two channels internally:

- **stdin/stdout** for input and output — the platform pipes the submission payload to stdin, reads the result from stdout
- **HTTP** for service calls — the SDK POSTs SdkMessage JSON to a bridge endpoint

The bridge URL is provided via environment variable:

```
VLINDER_BRIDGE_URL=http://host.containers.internal:9090
```

The bridge translates SDK calls to queue operations (via `handle_send` / ADR 044 message types). The agent never touches the queue directly.

### 4. `vlinder agent new` scaffolding

```
vlinder agent new my-agent --language python
```

Scaffolds from a language-specific template:

```
my-agent/
├── agent.toml           # agent manifest (name, model, storage)
├── Dockerfile           # language-appropriate base image + SDK install
├── src/
│   └── main.py          # hello-world agent, working out of the box
```

Templates are maintained as git repos per language. `vlinder agent new` clones the appropriate template, substitutes the agent name, and the result builds and runs immediately.

### 5. Supported languages (initial)

Start with **Python** — largest AI/ML ecosystem, lowest barrier to entry. Add others based on demand:

1. Python (first)
2. Node.js
3. Go
4. Rust

The protocol is simple enough (JSON + HTTP) that each SDK is under 200 lines. Maintenance burden is low.

## Consequences

**Positive:**
- Agent developers write domain logic, not infrastructure glue
- Hello-world works out of the box — `vlinder agent new` → build → run
- Protocol is language-agnostic — SDK is a thin wrapper, not a framework
- stdin/stdout + HTTP is universally supported — no exotic dependencies
- SDK surface is small (5 service domains) — easy to implement and maintain per language

**Negative:**
- Each new language requires a maintained SDK and template repo
- HTTP bridge adds a network hop per service call (mitigatable — bridge runs on same host)
- stdin/stdout limits agents to single request-response (no streaming yet)

## Deferred

- Streaming support (SSE, WebSocket, or chunked stdout)
- SDK package distribution (PyPI, npm, Go modules, crates.io)
- Agent manifest schema (`agent.toml` format)
- Template repo hosting and versioning
- SDK versioning and backwards compatibility
