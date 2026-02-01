# ADR 033: Agent URI Scheme

## Status

Proposed

## Context

Agents need a single identifier that encodes:
- Identity (who is this agent)
- Location (where to fetch it)
- Execution strategy (how to run/invoke it)
- Trust model (is it auditable)

## Decision

The `uri` field in agent.toml is the agent's canonical identifier. The scheme determines fetch and execution strategy.

### URI Schemes

| Scheme | Points to | Trust model | vlinder action |
|--------|-----------|-------------|----------------|
| `file://` | Local binary | You placed it there | Run locally |
| `git://` | Readable source (scripts) | Audit the code | Clone and run |
| `aws://` | Lambda function | AWS manages it | Invoke webhook |
| `gcp://` | Cloud Function | GCP manages it | Invoke webhook |
| `azure://` | Azure Function | Azure manages it | Invoke webhook |
| `https://` | OpenAPI spec | Spec is readable | Call per spec |

### Examples

```
file:///home/user/agents/pensieve.wasm
git://github.com/user/pensieve.git/agent.py
aws://lambda/us-east-1/pensieve
https://api.example.com/agents/pensieve/openapi.json
```

### Trust Rules

**Readable = can be in git:**
- Scripts (`.py`, `.sh`, `.ps1`) can be checked into git repos
- OpenAPI specs are readable JSON/YAML

**Binary = must be local or managed:**
- `.wasm`, `.exe` binaries are NOT checked into git
- Use `file://` for local binaries you built/obtained
- Use cloud schemes for managed deployments

### git:// Path Convention

For `git://` URIs, the path after `.git` specifies the file within the repo:

```
git://github.com/user/repo.git/path/to/agent.py
                              └─────────────────┘
                              path within repo
```

The file must be:
1. Readable (text, not binary)
2. Actually checked into the repo
3. Implements the vlinder agent contract (queue listener)

### Run vs Invoke

Two modes of operation:

**Run locally:**
- `file://` - vlinder executes the binary
- `git://` - vlinder clones and runs the script

**Invoke remotely:**
- `aws://`, `gcp://`, `azure://` - vlinder calls the webhook
- `https://` - vlinder calls per OpenAPI spec

### Agent Contract

All agents implement the same contract regardless of runtime:
- Listen to a queue (NATS)
- Process messages
- Respond to reply queue

vlinder provides template repos for each supported runtime/platform. Templates are the documentation of the contract per language.

### Manifest

```toml
name = "pensieve"
uri = "file:///home/user/agents/pensieve.wasm"
description = "Memory agent"

[requirements]
services = ["inference", "embedding"]
```

The `uri` replaces the old `source` and `code` fields - one field encodes everything.

## Consequences

- Agent identity is a URI, not just a name
- The scheme is the execution strategy
- No separate `runtime` or `code` fields needed
- Clear trust boundaries: readable vs managed
- Template repos define the contract per language/platform

## Templates

vlinder provides scaffolding for each supported runtime:

```
vlinder agent new --template=wasm-rust
vlinder agent new --template=python-script
vlinder agent new --template=lambda-python
vlinder agent new --template=openapi-node
```

Each template:
- Implements the vlinder agent contract for that runtime
- Has correct project structure
- Includes `agent.toml` with appropriate `uri` pattern
