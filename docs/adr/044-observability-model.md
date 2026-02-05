# ADR 044: Observability Model

## Status

Proposed

## Context

When debugging the distributed system with `nats sub vlinder.>`, messages are difficult to interpret:

```
[#8] Received on "vlinder.svc.kv.sqlite.put"
reply-to: reply-4a887371-c12d-4b36-a90c-7260837823e9
msg-id: acd0a0c5-65b6-445b-9d7a-591fd6947d8c

[#9] Received on "vlinder.agent.reply-4a887371-c12d-4b36-a90c-7260837823e9"
correlation-id: acd0a0c5-65b6-445b-9d7a-591fd6947d8c
```

**The root cause:** The current `Message` struct is generic and untyped:

```rust
pub struct Message {
    pub id: MessageId,
    pub payload: Vec<u8>,
    pub reply_to: String,
    pub correlation_id: Option<MessageId>,
}
```

Without knowing what **type** of interaction a message represents, you don't know what metadata to expect. Is this a request or a response? Which agent sent it? Which service is involved? The struct doesn't say.

This uncertainty, combined with opaque UUIDs everywhere, creates cognitive overload. You see a message and have no idea what it means or how it relates to other messages.

## Decision

### 1. Five Message Types

There are exactly five sender-receiver interactions in the system. Each message must explicitly declare which type it is:

| Type | Sender | Receiver | Purpose |
|------|--------|----------|---------|
| **Invoke** | Harness | Runtime | Start a submission |
| **Request** | Runtime | Service | Agent calls a service |
| **Response** | Service | Runtime | Service replies to agent |
| **Complete** | Runtime | Harness | Submission finished |
| **Call** | Runtime | Runtime | Agent invokes another agent (future) |

### 2. Each Type Has Explicit Fields

Each message type has its own struct with exactly the fields relevant to that interaction — no optional guessing:

**Invoke** (Harness → Runtime):
```
id, submission, harness, runtime, agent, payload, reply_to
```

**Request** (Runtime → Service):
```
id, submission, agent, service, backend, operation, sequence, payload, reply_to
```

**Response** (Service → Runtime):
```
id, submission, agent, service, backend, operation, sequence, payload, correlation_id
```

**Complete** (Runtime → Harness):
```
id, submission, agent, harness, payload, correlation_id
```

**Call** (Runtime → Runtime, future):
```
id, submission, caller_agent, target_runtime, target_agent, sequence, payload, reply_to
```

When you see a message, the type tells you exactly what fields are present and what they mean.

### 3. The Vocabulary: Eight Dimensions

The fields above draw from a vocabulary of eight observability dimensions:

| # | Dimension | Question | Examples |
|---|-----------|----------|----------|
| 1 | **Submission** | Which user-initiated request? | `sub-abc123` |
| 2 | **Harness** | Which entry point initiated it? | `cli`, `web`, `whatsapp` |
| 3 | **Agent** | Which agent is involved? | `pensieve`, `coder` |
| 4 | **Service** | What capability is needed? | `kv`, `vec`, `infer`, `embed` |
| 5 | **Backend** | Which implementation provides it? | `sqlite`, `ollama`, `s3`, `openai` |
| 6 | **Direction** | Request or response? | `req`, `res` |
| 7 | **Operation** | What action? | `put`, `get`, `search`, `store` |
| 8 | **Sequence** | Which interaction in this submission? | `1`, `2`, `3` |

Not all dimensions apply to all message types:

| Dimension | Invoke | Request | Response | Complete | Call |
|-----------|--------|---------|----------|----------|------|
| Submission | ✓ | ✓ | ✓ | ✓ | ✓ |
| Harness | ✓ | — | — | ✓ | — |
| Agent | ✓ | ✓ | ✓ | ✓ | ✓ (caller + target) |
| Service | — | ✓ | ✓ | — | — |
| Backend | — | ✓ | ✓ | — | — |
| Direction | implicit | ✓ | ✓ | implicit | ✓ |
| Operation | — | ✓ | ✓ | — | — |
| Sequence | — | ✓ | ✓ | — | ✓ |

### 4. Terminology

- **Submission**: A user-initiated request to invoke an agent. Replaces "job" which had batch-processing connotations.
- **Harness**: The entry point that initiated the submission. Examples: `cli`, `web`, `whatsapp`, `api`.
- **Service**: The capability being requested (what). Examples: `kv`, `vec`, `infer`, `embed`.
- **Backend**: The implementation providing the service (who/how). Examples: `sqlite`, `s3`, `ollama`, `openai`, `llama-cpp`.

### 5. Service + Backend Matrix

| Service | Backends | Operations |
|---------|----------|------------|
| `kv` | `sqlite`, `s3`, `minio`, `memory` | `put`, `get`, `list`, `delete` |
| `vec` | `sqlite-vec`, `pinecone`, `qdrant`, `memory` | `store`, `search`, `delete` |
| `infer` | `ollama`, `openai`, `llama-cpp`, `anthropic` | (bare — no sub-operation) |
| `embed` | `ollama`, `openai`, `llama-cpp` | (bare — no sub-operation) |

### 6. Routes Are Emergent

A **Route** is the path a submission takes through the system — the ordered sequence of nodes visited:

```
harness → pensieve → kv.sqlite.get → infer.ollama → kv.sqlite.put → harness
```

Routes matter for debugging, performance analysis, scaling decisions, and backpressure diagnosis.

However, routes cannot be predetermined:
- Orchestrators use small models — they know enough to route, not to plan entire executions
- Routing is probabilistic — the orchestrator makes its best guess
- Rerouting happens — strategies may change mid-execution
- User input variety is unpredictable

Therefore, routes are **computed from message data**, not encoded upfront. The typed messages provide the raw data; routes are reconstructed by grouping by submission and ordering by sequence.

## Design Considerations

### Subjects Are Free

NATS subjects are just strings — no infrastructure cost. One stream (`VLINDER`) captures all `vlinder.>` subjects. Rich subject names don't create queues or allocate resources.

### Every Sender Has Full Context

The WASM guest calls `send(payload)` but doesn't publish to NATS directly. The **runtime** intercepts and publishes. The runtime knows all relevant dimensions at send time.

### Subject Structure Is Observability, Not Deployment

Subject structure should be designed for **what we need to see** during debugging. Deployment adapts by choosing appropriate consumer filters based on machine capabilities.

### 7. Subject Structure

NATS subjects encode the message type and relevant dimensions in a consistent hierarchy:

```
vlinder.{submission}.{type}.{from}.{to}.{...details}.{seq}
```

**Reading order:**
1. System namespace (`vlinder`)
2. Which user request (`submission`)
3. What kind of message (`type`)
4. From → To (sender → receiver)
5. Details (operation, etc.)
6. Which instance (`seq`) — always last

**Subject patterns by message type:**

| Type | Subject Pattern |
|------|-----------------|
| **invoke** | `vlinder.{submission}.invoke.{harness}.{runtime}.{agent}` |
| **req** | `vlinder.{submission}.req.{agent}.{service}.{backend}.{operation}.{seq}` |
| **res** | `vlinder.{submission}.res.{service}.{backend}.{agent}.{operation}.{seq}` |
| **complete** | `vlinder.{submission}.complete.{agent}.{harness}` |
| **call** | `vlinder.{submission}.call.{caller_agent}.{target_runtime}.{target_agent}.{seq}` |

**Example flow:**

```
vlinder.sub-abc123.invoke.cli.wasm.pensieve
vlinder.sub-abc123.req.pensieve.kv.sqlite.get.1
vlinder.sub-abc123.res.kv.sqlite.pensieve.get.1
vlinder.sub-abc123.req.pensieve.infer.ollama.2
vlinder.sub-abc123.res.infer.ollama.pensieve.2
vlinder.sub-abc123.req.pensieve.kv.sqlite.put.3
vlinder.sub-abc123.res.kv.sqlite.pensieve.put.3
vlinder.sub-abc123.complete.pensieve.cli
```

**Common filters:**

| Want to see | Filter |
|-------------|--------|
| All messages for a submission | `vlinder.sub-abc123.>` |
| All requests for a submission | `vlinder.sub-abc123.req.>` |
| All invokes from CLI harness | `vlinder.*.invoke.cli.>` |
| All SQLite KV requests | `vlinder.*.req.*.kv.sqlite.>` |
| Everything | `vlinder.>` |

## Consequences

**Positive:**
- Message type is immediately visible in subject
- Submission groups all related messages
- From/To direction is explicit
- Sequence comes last (most granular identifier)
- Self-describing: you can read the subject and understand the interaction
- Easy filtering by any dimension

**Negative:**
- Longer subjects than current implementation
- Requires updating all message producers and consumers
- Harness must generate submission IDs
