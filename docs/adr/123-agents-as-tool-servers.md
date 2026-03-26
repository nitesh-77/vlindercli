# ADR 123: Agents as Tool Servers

**Status:** Draft

## Context

The current agent contract is a black box: the harness sends a payload to `/invoke`, the agent does whatever it wants internally, and returns a response. The harness has no visibility into what the agent *can* do, only what it *did* (via the DAG).

This limits:

- **Time travel granularity** — fork/replay only works at invoke/complete boundaries. You can't fork mid-execution to see what would have happened if a service call returned differently.
- **Fleet composition** — delegation is a special mechanism, not a natural composition of capabilities.
- **Human-in-the-loop** — requires custom implementation per agent, not a platform feature.
- **Discoverability** — no way to know what an agent can do without reading its code.

### Observation: tool calls are the universal primitive

Every interaction the agent has — service calls (KV, vector, inference), delegation to other agents, human approval — follows the same pattern: request with typed input, wait for typed output. These are all tool calls.

The durable execution checkpoint pattern ("yield an action, get a result") is precisely how MCP tool calls work. Developers already know this pattern from writing tool definitions for Claude/GPT.

### Observation: OpenAPI and MCP are two interfaces to the same thing

A web app with an OpenAPI spec and an MCP server backed by the same implementation expose the same capabilities through two interfaces — one for HTTP clients, one for LLM clients. With sufficient restrictions (statelessness, structured I/O), converting between OpenAPI and MCP is mechanical.

## Decision

### 1. Agents are HTTP apps with routes

The entry point is `POST /`. The agent decides what happens:

- **Simple agent** (echo): takes input, returns output. One route, works like today.
- **Multi-tool agent** (todoapp): handles `/` and picks tools internally based on input.
- **Tool server**: returns an OpenAPI spec / tool catalog. The harness picks which route to call.

All three work. The harness adapts based on what the agent exposes. Existing agents work unchanged.

### 2. Every external interaction is a tool call

Service calls, delegation, inference, external APIs (Jira, Stripe, Slack) — all follow the same pattern:

```
agent calls local provider endpoint → provider server intercepts →
harness records input in DAG → executes the real call →
harness records output in DAG → returns result to agent
```

There is no distinction between "platform service" and "external API." Each is a provider server backed by an OpenAPI spec + auth config. The platform ships with built-in providers (inference, storage, embedding). Third-party providers are registered the same way.

### 3. The harness drives execution

The harness is the execution engine. Agents are pure functions.

Instead of the agent driving its own loop (making HTTP calls, deciding when to delegate), the harness mediates every step. The agent yields actions, the harness executes them. Every step is recorded in the DAG.

This means:
- Fork from any step, at any depth of delegation
- Replay with substitution — swap any tool call's response
- Deterministic replay — re-drive execution from the DAG alone
- Full observability — every decision, every side effect, structured

### 4. Human-in-the-loop is a tool call

The agent yields "call `human.approve`" with a prompt. The harness routes to a human (via web UI, Slack, email, etc.), waits for the response, records it in the DAG, feeds it back to the agent.

From the agent's perspective, indistinguishable from any other tool call. From the DAG's perspective, a normal recorded step that can be forked/replayed.

### 5. MCP compatibility is generated, not primary

OpenAPI is the primary agent contract. MCP tool definitions are generated from the OpenAPI spec. If MCP changes or dies, the generator is updated or dropped — the core platform is unaffected.

Any MCP client (Claude, etc.) can drive a vlinder agent through the generated MCP interface. Any HTTP client can drive it through the OpenAPI interface. Same agent, two protocols.

### 6. Shared responsibility model

**Platform guarantees:**
- If your agent is stateless and uses platform stores, time travel works
- If you expose an OpenAPI spec, you get tool-level observability and composition
- Every interaction is recorded, forkable, replayable
- Orchestration, routing, state versioning handled

**Agent author guarantees:**
- No hidden state (use vlinder KV/vector stores)
- Idempotent routes (same input + same state = same output)

**Enforcement, not convention:**
- Lambda cold starts enforce statelessness — no discipline required
- Network restricted to provider servers — external calls go through the platform
- All state from the store, all side effects through the platform — idempotency is structural

### 7. External services are provider servers

A Jira API call is semantically identical to an OpenRouter inference call. Both are tool calls mediated by a provider server.

Registering a new external service: provide an OpenAPI spec + auth config. The platform generates the provider server. No custom integration code.

This means:
- Jira calls are replayable — fork from ticket creation, substitute a different result
- Stripe calls are observable — see exactly when charges were made
- Replay is safe — the platform mocks any external provider during replay

### 8. Services are agents, agents are services

The distinction between "agent" and "service" collapses. The KV store is an agent that exposes `get`, `put`, `list`, `delete` tools. Ollama is an agent that exposes `run`, `chat`, `generate` tools. A user-authored todoapp is an agent that exposes `add`, `list`, `done` tools.

The platform doesn't know the difference. Everything is registered in the catalog with a tool definition, everything receives calls from the harness, everything returns results.

This means:
- One registration model (not agents + services)
- One routing mechanism (not data-plane + service-plane)
- One recording format in the DAG

### 9. One message pair: call / result

If every interaction is a tool call, the fundamental message pair is:

- **Call**: "I want to invoke this tool with these args"
- **Result**: "Here's what came back"

The current six message types collapse:

| Current | In the tool model |
|---|---|
| Invoke (harness → agent) | Call (harness → agent's root tool) |
| Complete (agent → harness) | Result (agent → harness) |
| Request (agent → service) | Call (agent → service tool) |
| Response (service → agent) | Result (service → agent) |
| Delegate (agent → agent) | Call (agent → another agent's tool) |
| DelegateReply (agent → agent) | Result (agent → agent) |
| HITL (agent → human) | Call (agent → human approval tool) |

The `DataMessageKind` becomes `Call { caller, target, tool, sequence }` and `Result { caller, target, tool, sequence }`. The target type (agent, service, human) is just metadata on the tool definition, not a routing distinction.

## Open Questions

### Granularity guidance

The agent author decides tool granularity. A coarse agent (`POST /process` does 10 things) gets coarse time travel. A fine-grained agent (10 routes) gets fine-grained time travel but requires the caller to understand sequencing. How do we guide authors toward the right granularity?

### State between steps

If the agent yields after each tool call, where does intermediate state live? Options: the KV store (explicit), the DAG itself (conversation history), or agent memory (lost on restart). Lambda enforces the first two. Containers allow the third (a footgun).

### Inference routing

Is the LLM the caller (selecting tools) or a tool (the agent calls it)? Both patterns exist. The platform should support both — the LLM as orchestrator (via MCP) and the LLM as a service (via the inference provider).

## Consequences

- Any stateless HTTP app with an OpenAPI spec gets time travel for free
- Developers write normal web apps — no vlinder-specific SDK required
- MCP compatibility attracts the AI developer ecosystem
- Fleet composition is tool composition — no special delegation mechanism
- HITL is a platform feature, not a per-agent implementation
- External integrations follow the same pattern as built-in services
- The DAG becomes a typed sequence of tool calls, not opaque message blobs
