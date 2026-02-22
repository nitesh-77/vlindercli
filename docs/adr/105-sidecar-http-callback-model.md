# ADR 105: Sidecar HTTP Callback Model

**Status:** Accepted

## Context

The sidecar (ADR 084) mediates between the platform queue and the agent
container. On main, the sidecar drives the agent through a state machine
dispatch loop (`dispatch.rs`):

1. Sidecar sends `AgentEvent::Invoke` to `POST /handle`
2. Agent responds with an `AgentAction` (e.g. `KvGet`, `Infer`, `Complete`)
3. Sidecar executes the action via `SdkContract`, wraps the result in
   another `AgentEvent`, POSTs back to `/handle`
4. Repeat until `AgentAction::Complete`

This works but has two costs:

1. **Every agent must implement the dispatch protocol.** The agent SDK
   needs to parse `AgentEvent`, build `AgentAction`, and manage a
   `/handle` endpoint that round-trips through the full enum on every
   service call. The protocol is a 10-variant enum with serialized state
   threaded through every variant.

2. **Two-sided schema coupling.** `AgentAction` and `AgentEvent` must
   match exactly between the sidecar (Rust) and every agent SDK (Python,
   etc.). Adding a service means adding variants to both enums, updating
   both serialization formats, and coordinating the rollout.

## Decision

Invert the control flow. The sidecar POSTs once to `/invoke` with the
input payload and blocks for the final response. The agent calls back to
the sidecar's HTTP server for platform services as needed.

### Agent-facing API (sidecar → agent)

```
POST /invoke    — send input, block for final output
GET  /health    — readiness check before entering dispatch loop
```

### Platform services API (agent → sidecar, port 9000)

```
POST /services/kv/get
POST /services/kv/put
POST /services/kv/delete
POST /services/vector/store
POST /services/vector/search
POST /services/vector/delete
POST /services/infer
POST /services/embed
```

Each endpoint accepts JSON matching the existing service payload types
(`KvGetRequest`, `InferRequest`, etc.) and returns the worker response.
The sidecar handles queue routing, sequence tracking, and state cursor
management — the agent never sees these concerns.

### Implementation

The HTTP callback server runs on a background thread using `tiny_http`.
The main thread runs the dispatch loop: poll queues, POST to `/invoke`,
block for response. Two threads, same sync architecture as before.

`dispatch.rs` is removed. `AgentAction`, `AgentEvent`, and `SdkContract`
are dead code.

`QueueBridge` moves from vlinder-core into vlinder-sidecar. It's the
implementation behind the HTTP callback endpoints — sidecar-specific,
not a core domain concept.

### Wire protocol note

The KV worker expects the `content` field in PUT requests to be
base64-encoded (binary safety over JSON). The sidecar encodes on PUT
and the worker decodes on store. GET responses return raw decoded bytes.
No other service uses base64.

## Consequences

- Agent SDKs become HTTP clients. The todoapp SDK is a Python class
  with `requests.post()` calls to `localhost:9000`.
- Adding a platform service means adding one HTTP endpoint and one
  `QueueBridge` method. No enum variants, no agent-side changes.
- The sidecar's `/invoke` call blocks for the full duration of agent
  execution. This is fine — one invocation at a time per pod.
