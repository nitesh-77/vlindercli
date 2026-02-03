# Request Flow

How a user request travels through the system.

---

## Overview

```
User ─► CLI ─► Harness ─► Registry ─► Queue ─► Runtime ─► Agent
                                        │
                                        ├─► Workers (inference, embedding, storage)
                                        │
                                        └─► Response back via reply queue
```

---

## Step-by-Step Flow

### 1. User runs an agent

```bash
vlinder agent run -p agents/pensieve/
```

### 2. CLI → Harness

- CLI parses arguments, calls `Daemon::run()`
- Daemon owns Harness, which provides the API surface
- Harness calls `deploy_from_path("agents/pensieve/")`

### 3. Harness → Registry

- Harness loads `agent.toml` manifest
- Validates agent requirements against Registry capabilities
- Registers agent: `registry.register_agent(agent)`
- Creates job: `registry.create_job(agent_id, input)` → returns `JobId`

### 4. Harness → Queue

- Harness queues message to agent's input queue
- Message contains: payload, reply_to queue, request ID

```
Queue: "file:///path/to/agent.wasm"
Message: { payload: "user input", reply_to: "job-xyz-reply", id: 123 }
```

### 5. Runtime → Agent

- `WasmRuntime.tick()` polls Registry for WASM agents
- Finds message in agent's queue
- Spawns WASM execution in background thread
- Provides host functions: `send()`, `get_prompts()`

### 6. Agent → Workers (via Queue)

When agent needs inference, embedding, or storage:

```
Agent calls send({ op: "infer", model: "phi3", prompt: "..." })
         │
         ▼
    Queue: "infer"
         │
         ▼
    InferenceServiceWorker.tick()
         │
         ├── Validates agent declared this model
         ├── Gets/loads engine from cache
         └── Runs inference, sends response to reply queue
```

**Queue names by service:**

| Service | Queue(s) |
|---------|----------|
| Inference | `infer` |
| Embedding | `embed` |
| Object Storage | `kv-get`, `kv-put`, `kv-delete`, `kv-list` |
| Vector Storage | `vector-store`, `vector-search`, `vector-delete` |

### 7. Response Flow

```
Worker ─► reply queue ─► Agent (resumes) ─► Agent output
                                               │
                                               ▼
Runtime ─► job reply queue ─► Harness ─► updates job status ─► User sees result
```

---

## Sequence Diagram

```
User        CLI       Harness     Registry      Queue       Runtime      Worker
 │           │           │           │            │            │            │
 │──run─────►│           │           │            │            │            │
 │           │──deploy──►│           │            │            │            │
 │           │           │─register─►│            │            │            │
 │           │           │◄──ok──────│            │            │            │
 │           │           │create_job►│            │            │            │
 │           │           │◄──job_id──│            │            │            │
 │           │           │─────────send_message──►│            │            │
 │           │           │           │            │            │            │
 │           │           │           │     [tick] │◄──poll─────│            │
 │           │           │           │            │──message──►│            │
 │           │           │           │            │            │──execute───►
 │           │           │           │            │            │            │
 │           │           │           │            │   [agent calls send()]  │
 │           │           │           │            │◄───────────│            │
 │           │           │           │            │────────────────────────►│
 │           │           │           │            │◄────────────────────────│
 │           │           │           │            │───────────►│            │
 │           │           │           │            │            │◄───────────│
 │           │           │           │            │            │            │
 │           │           │           │            │◄──response─│            │
 │           │◄─────────poll_result──│            │            │            │
 │◄──result──│           │           │            │            │            │
```

---

## Key Points

1. **Everything is async via queues** — no direct function calls between components
2. **Registry is source of truth** — agents, jobs, capabilities all tracked there
3. **Workers lazy-load backends** — first request triggers engine/storage initialization
4. **Agent isolation** — each agent has its own storage, keyed by agent_id
5. **Reply queues** — every request includes a reply_to for the response

---

## Related Documentation

- [Domain Model](DOMAIN_MODEL.md) — types and traits
- [ADR 018](adr/018-protocol-first-architecture.md) — queue-based architecture
- [ADR 031](adr/031-vlinderd-as-runtime-registry.md) — registry and daemon design
