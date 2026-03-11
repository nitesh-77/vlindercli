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

### 1. User deploys and runs an agent

```bash
vlinder agent deploy -p agents/todoapp/
vlinder agent run todoapp
```

### 2. CLI → Harness

- CLI parses arguments, connects to the Harness via gRPC
- Harness manages sessions, submission chaining, and state tracking

### 3. Harness → Registry

- Harness loads `agent.toml` manifest
- Validates agent requirements against Registry capabilities
- Registers agent: `registry.register_agent(agent)`
- Creates job: `registry.create_job(agent_id, input)` → returns `JobId`

### 4. Harness → Queue

- Harness publishes an `InvokeMessage` to the agent's NATS subject
- Message contains: payload, session, submission, timeline, state hash

### 5. Runtime → Agent

- `ContainerRuntime.tick()` polls the invoke queue
- Finds message, dispatches to the agent's container via HTTP `POST /invoke`
- The agent runs as an OCI container with a sidecar for service access

### 6. Agent → Workers (via sidecar DNS)

When the agent needs inference, embedding, or storage:

```
Agent calls http://ollama.vlinder.local:3544/v1/chat/completions
         │
         ▼
    Sidecar intercepts, publishes RequestMessage to NATS
         │
         ▼
    OllamaWorker.tick() picks up the request
         │
         ├── Validates agent declared this model
         ├── Runs inference via Ollama HTTP API
         └── Publishes ResponseMessage to reply subject
```

**NATS subjects by service type:**

| Service | Subject pattern |
|---------|----------------|
| Inference (Ollama) | `request.infer.ollama` |
| Inference (OpenRouter) | `request.infer.openrouter` |
| Embedding | `request.embed.ollama` |
| Object Storage | `request.kv.sqlite` |
| Vector Storage | `request.vec.sqlite` |

### 7. Response Flow

```
Worker ─► ResponseMessage ─► Sidecar ─► Agent (resumes)
                                              │
                                              ▼
Runtime ─► CompleteMessage ─► Harness ─► updates job status ─► User sees result
```

---

## Sequence Diagram

```
User        CLI       Harness     Registry      Queue       Runtime      Worker
 │           │           │           │            │            │            │
 │──run─────►│           │           │            │            │            │
 │           │──gRPC────►│           │            │            │            │
 │           │           │─register─►│            │            │            │
 │           │           │◄──ok──────│            │            │            │
 │           │           │create_job►│            │            │            │
 │           │           │◄──job_id──│            │            │            │
 │           │           │──InvokeMessage────────►│            │            │
 │           │           │           │            │            │            │
 │           │           │           │     [tick] │◄──poll─────│            │
 │           │           │           │            │──message──►│            │
 │           │           │           │            │            │─POST /invoke─►
 │           │           │           │            │            │            │
 │           │           │           │            │  [agent calls sidecar]  │
 │           │           │           │            │◄───────────│            │
 │           │           │           │            │────────────────────────►│
 │           │           │           │            │◄────────────────────────│
 │           │           │           │            │───────────►│            │
 │           │           │           │            │            │◄───────────│
 │           │           │           │            │            │            │
 │           │           │           │            │◄──Complete──│            │
 │           │◄─────────poll_result──│            │            │            │
 │◄──result──│           │           │            │            │            │
```

---

## Key Points

1. **Everything flows through NATS:** no direct function calls between components
2. **Registry is source of truth:** agents, models, jobs, capabilities all tracked there
3. **Workers lazy-load backends:** first request triggers engine/storage initialization
4. **Agent isolation:** each agent has its own storage, keyed by agent_id
5. **Sidecar DNS:** agents access services via `*.vlinder.local:3544` hostnames

---

## Related Documentation

- [Domain Model](DOMAIN_MODEL.md): types and traits
- [ADR 018](adr/018-protocol-first-architecture.md): queue-based architecture
- [ADR 031](adr/031-vlinderd-as-runtime-registry.md): registry and daemon design
