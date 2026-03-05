# ADR 111: Step Function Execution Mode

**Status:** Draft

## Context

Agents currently run in "unmanaged" mode (ADR 105): the sidecar POSTs to
`/invoke`, the agent drives its own tool-use loop by calling provider
hostnames directly, and the sidecar blocks until the final response. The
agent's intermediate execution state lives on its call stack — when
`/invoke` returns, that state is gone.

This works well for the happy path. But it breaks time travel.

Consider a multi-step agent that calls inference, gets tool_use blocks,
executes tools, feeds results back, and calls inference again. The
platform sees the individual service calls (each is a RequestMessage with
a sequence number in the DAG), but it cannot *resume* from an
intermediate step — because the agent's execution state was never
persisted. The conversation repo records every service call, but there's
no way to go back to step 3 and re-deliver a tool result.

ADR 075 proposed solving this with a custom state machine protocol:
`AgentEvent`/`AgentAction` enums with 10+ variants, a `/handle` endpoint,
and explicit `ctx.state` threading. ADR 105 rejected that design because
of two-sided schema coupling — every new service required new enum
variants in both the sidecar (Rust) and every agent SDK.

The insight: LLM APIs are already step functions. The OpenAI Chat
Completion and Anthropic Messages APIs follow the exact pattern we need:

```
messages → response (may contain tool_use) → execute tools →
messages + tool_results → response → ... → final response (no tool_use)
```

Each step is a complete, serializable state (the messages array). The
protocol is standardized. Agent authors already think in these terms.

## Decision

**Add a step function execution mode where the sidecar drives the
tool-use loop using OpenAI/Anthropic message format.**

This is not a new RuntimeType — agents still run on Container or Lambda.
It's a change in how the sidecar (or Lambda adapter) drives the agent.

### Manifest flag

```toml
name = "finqa"
description = "Financial Q&A agent"
runtime = "container"
executable = "localhost/finqa:latest"
execution_mode = "step"            # NEW — default: "unmanaged"

[requirements.services.infer]
provider = "openrouter"
protocol = "anthropic"
models = ["anthropic/claude-3.5-sonnet"]
```

`execution_mode` has two values:
- `"unmanaged"` (default) — current behavior. Sidecar POSTs `/invoke`,
  agent drives its own loop. Backward compatible.
- `"step"` — sidecar drives the tool-use loop. Agent is a thin wrapper
  around a single LLM call per step.

### Agent contract in step mode

The agent exposes the same HTTP server but with a different contract:

```
POST /invoke    — receives messages array, returns LLM response
GET  /health    — readiness check (unchanged)
```

**Request** (sidecar → agent): JSON messages array in the protocol
declared by `requirements.services.infer.protocol`:

```json
{
  "messages": [
    {"role": "user", "content": "What were ACME's Q3 earnings?"}
  ]
}
```

On subsequent steps, the sidecar appends tool results:

```json
{
  "messages": [
    {"role": "user", "content": "What were ACME's Q3 earnings?"},
    {"role": "assistant", "content": null, "tool_use": [...]},
    {"role": "tool", "tool_use_id": "xyz", "content": "..."}
  ]
}
```

**Response** (agent → sidecar): The raw LLM response, which either:
- Contains `tool_use` blocks → sidecar executes them, appends results,
  calls `/invoke` again
- Contains only text content → sidecar extracts final output, sends
  CompleteMessage

The agent's job is minimal: receive messages, call the LLM, return the
response. System prompt, tool definitions, model selection — all owned by
the agent. The sidecar owns the loop.

### Sidecar loop (step mode)

```
1. Receive InvokeMessage from queue
2. Build initial messages: [{"role": "user", "content": payload}]
3. POST /invoke → agent with {messages}
4. Agent calls LLM, returns response
5. Parse response:
   a. Has tool_use blocks?
      - For each tool_use: execute via provider server infrastructure
        (same queue.call_service() path as unmanaged mode)
      - Append assistant message + tool result messages
      - Record step as DAG node (messages array = the state)
      - GOTO 3
   b. No tool_use (final response)?
      - Extract text content as output
      - Build CompleteMessage with final state
      - Send to queue
```

Each iteration of this loop is a **step**. Each step produces a git
commit in the conversation DAG containing:
- The full messages array at that point
- The tool calls made and their results
- The state hash

This is what makes mid-execution time travel possible: any step can be
checked out and resumed by re-delivering the messages array up to that
point.

### Tool execution

Tool calls in the LLM response map to provider server operations. The
sidecar already has the infrastructure for this — `build_hosts()`,
`match_route()`, `forward_to_queue()`, `call_service()`. In step mode,
the sidecar calls these directly instead of the agent calling them via
HTTP.

The tool name in the LLM response maps to a provider operation:
- `kv_get`, `kv_put` → sqlite-kv provider
- `vector_search`, `vector_store` → sqlite-vec provider
- `delegate` → runtime.vlinder.local delegation

The mapping between LLM tool names and provider operations is defined by
the agent (in its tool definitions to the LLM). The sidecar needs a
registry of tool-name → provider-route to execute them. This can be:
- Convention-based (tool names match provider operation names)
- Declared in agent.toml under a `[tools]` section
- Returned by the agent via a `/tools` endpoint at startup

Decision on tool mapping is deferred to implementation — start with
convention-based and add explicit mapping if needed.

### What about inference calls?

In unmanaged mode, the agent calls the inference provider directly (e.g.,
`POST http://openrouter.vlinder.local:3544/v1/chat/completions`). In step
mode, the agent still calls inference directly — the sidecar only drives
the *outer* tool-use loop.

The agent owns the LLM call because it owns:
- System prompt and tool definitions
- Model selection and parameters
- Response parsing and any agent-specific logic

The sidecar owns the tool *execution* because it owns:
- Queue routing (RequestMessage → NATS → worker → ResponseMessage)
- State tracking (sequence numbers, state cursor)
- DAG recording (each step = a commit)

### Protocol support

The manifest's `requirements.services.infer.protocol` field already
declares whether the agent speaks OpenAI or Anthropic format. The sidecar
uses this to know how to parse tool_use blocks from the LLM response:

- **OpenAI**: `response.choices[0].message.tool_calls[]` with
  `{id, function: {name, arguments}}`
- **Anthropic**: `response.content[]` where `type == "tool_use"` with
  `{id, name, input}`

Both formats carry the same semantic information. The sidecar needs a
small parser for each.

### Coexistence

Both modes use the same:
- Container/Lambda runtimes (RuntimeType unchanged)
- Provider server infrastructure
- Queue protocol (5 message types)
- DAG recording
- Health check endpoint

The only difference is who drives the loop:
- Unmanaged: agent drives, sidecar blocks on `/invoke`
- Step: sidecar drives, agent handles one LLM call per `/invoke`

The sidecar checks `execution_mode` from the agent's registry entry
and branches at the top of `handle_invoke()`.

### State and time travel

In step mode, each step's state is the messages array — a complete,
serializable representation of the conversation so far. This is
fundamentally different from unmanaged mode where intermediate state
lives on the agent's call stack.

To repair from step N:
1. Checkout the DAG node at step N
2. Read the messages array from that commit
3. Re-execute the tool call that failed (service is back)
4. Append the tool result to messages
5. POST /invoke → agent with updated messages
6. Continue the loop from step N+1

No replay of prior steps. No reconstruction of agent state. The messages
array *is* the state.

## Consequences

- Time travel works for step-mode agents — every intermediate step is
  independently addressable and resumable
- Agent code becomes simpler — just wrap an LLM call, no loop management
- No custom enum coupling (the ADR 075 problem) — uses standardized
  LLM message format that agents already speak
- Unmanaged mode is untouched — existing agents keep working
- The sidecar gains a second code path (step loop) — bounded complexity
  since it reuses existing provider server infrastructure
- Agent authors choose their mode — unmanaged for full control, step for
  platform-managed time travel
- Tool mapping needs a convention or declaration mechanism

## Scope

### This ADR

- Define step function as an execution mode flag, not a runtime type
- Define the sidecar-driven loop using OpenAI/Anthropic message format
- Define how tool_use blocks map to provider operations
- Define per-step DAG recording for time travel

### Deferred

- **Timeline repair for step mode** — the actual `vlinder timeline repair`
  implementation that uses step checkpoints
- **Streaming** — step mode naturally supports streaming (SSE from agent
  to sidecar per step) but this is not needed initially
- **Max steps / circuit breaker** — safety limit on loop iterations
- **Parallel tool calls** — executing multiple tool_use blocks concurrently
  (start sequential, optimize later)
- **Tool declaration format** — if convention-based mapping proves
  insufficient, formalize a `[tools]` manifest section
