# ADR 048: Agent Registry Identity

## Status

Accepted

## Context

Agent `id` currently serves two conflicting roles:

1. **Registry identity** — the key used to look up agents, store them, reference them in messages
2. **Executable reference** — `container://localhost/echo-container:latest` tells the runtime how to run the agent

This means changing where an agent runs (container → Lambda → WASM) changes its identity, breaking all references (jobs, messages, queue subjects).

Models already solved this separation:
- `model.id` = `<registry>/models/<name>` (assigned by registry during registration)
- `model.model_path` = `ollama://localhost:11434/phi3:latest` (artifact locator)

Agents should follow the same pattern.

## Decision

Separate agent identity from executable reference.

### Identity

- `agent.id` is registry-assigned: `<registry_id>/agents/<name>`
- Format matches models: `http://127.0.0.1:9000/agents/echo-container`
- Before registration, agents carry a placeholder: `pending-registration://agents/<name>`
- The `name` field in the manifest is user-controlled and becomes the primary key
- Agent names are unique within a registry — duplicate registration is an error

### Executable and Runtime

- New field `executable` holds the artifact reference — a plain image ref, file path, or ARN
- New field `runtime` declares the runtime type explicitly
- `container://` was a synthetic URI scheme used for dispatch — replaced by explicit `runtime`
- `executable` values are now native references for their ecosystem:
  - `localhost/echo-container:latest` — OCI image (local or remote registry)
  - `ghcr.io/user/my-agent:latest` — OCI image from HTTPS registry
  - `./agent.wasm` — future WASM support
  - `arn:aws:lambda:...` — future Lambda support
- Runtime dispatch uses `agent.runtime`, not URI scheme parsing

### Manifest change

Before:
```toml
name = "echo-container"
id = "container://localhost/echo-container:latest"
```

After:
```toml
name = "echo-container"
runtime = "container"
executable = "localhost/echo-container:latest"
```

### Registration

- Registry assigns `<registry_id>/agents/<name>` during `register_agent()`
- Duplicate name → error (not silent overwrite)
- Queue routing uses `agent.name` directly (resolves existing TODO smell in `agent_routing_key()`)

## Consequences

- Agent identity is stable across runtime changes
- Aligns with model identity pattern — one pattern for all registry resources
- Breaking manifest schema change (`id` → `runtime` + `executable`)
- `executable` values are native to their ecosystem — no synthetic URI wrapping
- Queue routing simplified to use name directly
