# ADR 075: State Machine Agents

## Status

Draft

## Context

Agents are OCI containers (ADR 046) that expose a synchronous HTTP endpoint. The platform calls `POST /` with the user's message. The agent blocks on bridge calls (`POST /kv/get`, `POST /infer`, etc.) and returns a plain-text response. All intermediate state lives on the call stack — when the handler returns, that state is gone.

This worked well for the happy path. But it breaks time travel.

State now flows through all five message types (Invoke, Request, Response, Complete, Delegate). The conversation repo records every message as a git commit. We can `checkout` any commit and see the exact state of the world at that point. But we can't *resume* from that point — because the agent's execution state was never persisted. It lived on the call stack and died when the handler returned.

Consider a roundtrip where ollama was briefly down:

```
c1  invoke:   cli → agent
c2  request:  agent → kv.sqlite       kv-get
c3  response: kv.sqlite → agent       (data)
c4  request:  agent → infer.ollama    (ollama down)
c5  response: infer.ollama → agent    [error]
c6  complete: agent → cli             [error]
```

At c3, the agent had read from storage and was about to call inference. That intermediate result was a local variable. After c6, it's gone. We can't go back to c3 and re-deliver the inference response — because there's no agent waiting for it.

The forcing function is time travel. The fix is making the agent a state machine.

## Decision

**Agents are state machines. The platform drives the execution loop.**
The architectural shift: Currently, agents call into the platform (outbound HTTP to bridge server). The new model inverts this — the platform calls the agent repeatedly (POST /handle), and the agent returns what it needs as a JSON action. This is the "inversion of control" pattern: the platform becomes the driver, the agent becomes a pure function (Event, State) → (Action, State). This makes every intermediate step capturable — the key to time travel debugging.

The SDK inverts how agents interact with the platform. Previously, each service call was a synchronous HTTP request to the bridge server — the agent's call stack held the intermediate state. Now, each handler must explicitly return an action and store its progress in ctx.state. This is the classic "continuation-passing style" pattern: instead of blocking on I/O, you say "here's what I need and here's my state — call me back with the result." The tradeoff is more explicit flow control, but the payoff is that every step is independently addressable for replay/debugging.

### Container contract

The endpoint changes from `POST /` (plain text) to `POST /handle` (JSON):

**Request** (platform → agent):
```json
{
  "type": "invoke",
  "input": "user message",
  "state": {}
}
```

Or, for a service response:
```json
{
  "type": "kv.get.response",
  "payload": "...",
  "state": {"key": "value"}
}
```

**Response** (agent → platform):

Service request:
```json
{
  "action": "request",
  "service": "kv",
  "op": "get",
  "params": {"path": "/data.json"},
  "state": {"key": "value"}
}
```

Completion:
```json
{
  "action": "complete",
  "payload": "result text",
  "state": {"key": "value"}
}
```

Delegation:
```json
{
  "action": "delegate",
  "agent": "other-agent",
  "input": "...",
  "state": {"key": "value"}
}
```

### Two kinds of state

| | `ctx.state` | KV store |
|---|---|---|
| Scope | Within one roundtrip | Across roundtrips |
| Managed by | Platform | Agent (via service calls) |
| Persisted | In git commit (per step) | In SQLite (content-addressed) |
| Purpose | Execution state (local variables) | Persistent data |
| Time travel | Platform delivers the right state | State: trailer points to the right version |

`ctx.state` is a JSON object. The platform passes it in with each `POST /handle` call. The agent returns it (possibly modified) with each response. The platform stores it between steps and records it in the git commit.

KV storage still works as before — the agent returns `{"action": "request", "service": "kv", ...}` and the platform sends the request to the KV worker.

### Platform drives the loop

The runtime orchestrates each roundtrip:

```
1. Receive InvokeMessage from harness
2. POST /handle → agent: {type: "invoke", input: "...", state: {}}
3. Agent returns: {action: "request", service: "kv", op: "get", ...}
4. Runtime builds RequestMessage, sends to NATS
5. Worker processes, sends ResponseMessage
6. Runtime receives response
7. POST /handle → agent: {type: "kv.get.response", payload: "...", state: {...}}
8. Agent returns: {action: "request", service: "infer", ...}
9. ... repeat until ...
10. Agent returns: {action: "complete", payload: "..."}
11. Runtime builds CompleteMessage, sends to harness
```

Each step is a git commit. The agent never calls the bridge. The platform sends requests on the agent's behalf. The bridge (as an HTTP server that agents call) is no longer needed for the agent → platform direction. The bridge concept inverts: the runtime calls the agent, not the other way around.

### Agent SDK pattern

The template repos (`vlinder-agent-python`, etc.) scaffold the handler pattern using the web framework the developer already knows. A thin `Agent` class provides handler registration and typed contract methods from `AgentBridge` (ADR 074) — `kv_get`, `kv_put`, `infer`, `embed`, `vector_store`, `vector_search`, `delegate`, `wait`:

```python
from vlinder import Agent
from flask import Flask

app = Flask(__name__)
agent = Agent(app)

@agent.on_invoke
def start(ctx):
    ctx.kv_get(path="/data.json")

@agent.on_kv_get
def got_data(ctx):
    ctx.infer(model="phi3", prompt=ctx.input)

@agent.on_infer
def got_result(ctx):
    ctx.kv_put(path="/data.json", content=ctx.response)

@agent.on_kv_put
def saved(ctx):
    ctx.complete(ctx.state["result"])

app.run(host="0.0.0.0", port=8080)
```

The `Agent` class:
- Registers handlers as routes on the web framework (e.g., `/handle/invoke`, `/handle/kv_get`)
- Provides typed `ctx` methods matching the `AgentBridge` contract — no strings, no raw JSON
- Each language template uses its idiomatic web framework (Flask, Express, Gin, Spring Boot, .NET minimal API)

Each handler is one step in the contract. The handler names (`on_kv_get`, `on_infer`) match the `ctx` methods (`ctx.kv_get()`, `ctx.infer()`). One vocabulary — the `AgentBridge` trait — across decorators, context methods, and HTTP routes.

| Python SDK | AgentBridge trait |
|---|---|
| `ctx.kv_get(path: str) -> bytes` | `fn kv_get(&self, path: &str) -> Result<Vec<u8>, String>` |
| `ctx.kv_put(path: str, content: str) -> None` | `fn kv_put(&self, path: &str, content: &str) -> Result<(), String>` |
| `ctx.kv_list(prefix: str) -> list[str]` | `fn kv_list(&self, prefix: &str) -> Result<Vec<String>, String>` |
| `ctx.kv_delete(path: str) -> bool` | `fn kv_delete(&self, path: &str) -> Result<bool, String>` |
| `ctx.vector_store(key: str, vector: list[float], metadata: str) -> None` | `fn vector_store(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String>` |
| `ctx.vector_search(vector: list[float], limit: int) -> list[VectorMatch]` | `fn vector_search(&self, vector: &[f32], limit: u32) -> Result<Vec<VectorMatch>, String>` |
| `ctx.vector_delete(key: str) -> bool` | `fn vector_delete(&self, key: &str) -> Result<bool, String>` |
| `ctx.infer(model: str, prompt: str, max_tokens: int) -> str` | `fn infer(&self, model: &str, prompt: &str, max_tokens: u32) -> Result<String, String>` |
| `ctx.embed(model: str, text: str) -> list[float]` | `fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, String>` |
| `ctx.delegate(agent: str, input: str) -> str` | `fn delegate(&self, target_agent: &str, input: &str) -> Result<String, String>` |
| `ctx.wait(handle: str) -> bytes` | `fn wait(&self, handle: &str) -> Result<Vec<u8>, String>` |

### Time travel

At c3, the git commit contains:
- The agent's `ctx.state` at that step
- The agent's last action (a request to infer.ollama)
- The request payload

To repair from c3:

```bash
vlinder timeline checkout c3
vlinder timeline repair
```

Repair:
1. Reads c3's commit — finds the pending request and its payload
2. Sends that RequestMessage to NATS
3. Worker processes (service is back), sends ResponseMessage
4. Reads c3's `ctx.state` from the commit
5. POST /handle → agent with the response and the persisted state
6. Agent returns next action
7. Platform continues the loop normally
8. Each new step is a commit: c4', c5', c6', c7', c8'

Clean timeline. No duplicate service calls. No replay tricks. The platform delivers the right event to the right state.

### Purity and determinism

Agents are pure functions. The only operations available are the 11 methods on `AgentBridge` — all platform-managed, all internal. Agents cannot make external HTTP calls, write to disk, or produce side effects outside the platform boundary.

External side effects (email, Slack, webhooks, databases) are driven by NATS subscribers outside the system boundary. Subscribers own their own idempotency. The agent says "I'm done, here's the result." What happens next is not the agent's concern.

This purity is what makes time travel safe. There are no external side effects to undo or compensate. Repair replays platform service calls — all versioned, all reproducible.

With local models (pinned, hash-verified weights) and explicit RNG seeds, the system achieves full determinism. Same weights + same input + same seed = same output. `vlinder replay <trace_id>` produces identical results. Every time.

### What changes

- Container contract: `POST /` (plain text) → `POST /handle` (JSON)
- Agent code: one blocking function → event handlers
- Template repos: inline `bridge_call()` → `Agent` class with `@on()` decorators
- Runtime: waits for agent → drives the loop
- Bridge: agent calls bridge → runtime calls agent (direction inverts)

### What stays the same

- OCI containers, port 8080, Podman (ADR 046)
- `agent.toml` manifest format
- `vlinder agent new` scaffolding flow
- NATS as backbone
- Workers (inference, KV, vector) — unchanged
- Message types (Invoke, Request, Response, Complete, Delegate)
- Health check endpoint
- The five-message protocol (ADR 018)
- State store (ADR 055) — content-addressed KV versioning
- Git conversation repo (ADR 064, 068)

## Consequences

- Time travel works — repair any intermediate step by delivering the right event to the right state
- Every step is independently addressable in git — checkout any commit, see exact agent state
- Agents are simpler to reason about — each handler does one thing
- The platform has full visibility into agent execution — no opaque blocking calls
- Template repos control the DX — handlers, not switch statements
- Existing agents need migration — the contract changes
- Sequential code becomes event handlers — developers think in transitions, not procedures
- Concurrent service calls need a batch action type (deferred)

## Scope

### This ADR

- Define the state machine contract (POST /handle, JSON in/out)
- Define `ctx.state` semantics (within-roundtrip, platform-managed)
- Define the platform loop (runtime drives)
- Commit format: `ctx.state` recorded in each git commit

### Follow-on ADRs

- **Timeline repair** — the command that uses state machine checkpoints for time travel
- **Batch actions** — agent returns multiple requests for parallel execution
- **Migration guide** — how to convert existing synchronous agents
- **SDK libraries** — extract the `Agent` class into publishable packages per language
