# ADR 038: Logical vs Physical Manifest Schema

## Status

Accepted (validated in dca944b)

## Context

Current model and agent manifests conflate two concerns:

1. **Logical identity**: What an entity is (name, type, capabilities)
2. **Physical location**: Where to find artifacts (file paths, URIs)

This creates several problems:

### Problem 1: Identity Tied to Location

Model identity is currently the path to weight files:

```toml
name = "phi3"
id = "file:///Users/alice/models/phi-3-mini-Q4.gguf"
```

If Alice shares this manifest with Bob, it breaks. Moving the file breaks the identity. The model's identity should be independent of where its weights happen to be stored.

### Problem 2: Models as User-Brought Resources

The current design treats models as something the user provides via file paths. But models are first-class domain entities that should be:
- Cataloged and managed by the system
- Referenced by logical name, not file location
- Shareable across agents without path duplication

### Problem 3: Non-Portable Agent Manifests

Agent manifests hardcode environment-specific details:

```toml
id = "agent.wasm"
object_storage = "sqlite:///Users/alice/data/notes.db"

[requirements.models]
phi3 = "file://./models/phi3.toml"
```

This manifest only works on Alice's machine. The agent's logical requirements (needs phi3, needs storage) are mixed with physical bindings (specific file paths).

### Problem 4: Registry Opens New Possibilities

With a registry (ADR 031), we no longer need user-provided URIs for identity. The registry can:
- Issue canonical identifiers for resources
- Resolve logical names to physical locations
- Be the single source of truth for what exists in the system

## Decision

**Separate logical identity from physical location. The registry owns identity.**

### Principle

Manifests declare *what* an entity is and *what* it needs. The registry assigns identity and resolves physical locations.

### Resource Identity

All resources (models, agents) are identified by registry-issued URIs:

```
registry://<registry-authority>/<resource-type>/<name>
```

Examples:
- `registry://localhost:9000/models/phi3`
- `registry://localhost:9000/agents/pensieve`

The registry maintains mappings from these identifiers to physical artifacts:

```
registry://localhost:9000/models/phi3
  → file:///var/vlinder/models/phi-3-mini-Q4.gguf

registry://localhost:9000/agents/pensieve
  → file:///var/vlinder/agents/pensieve/agent.wasm
```

### Registration Flow

1. User provides manifest (name, type, capabilities) + physical artifact
2. Registry validates and stores the mapping
3. Registry returns the canonical resource ID
4. All subsequent references use the registry ID

```
# Register a model
vlinder model register ./phi3.toml --weights ./phi-3-mini.gguf
→ Registered: registry://localhost:9000/models/phi3

# Register an agent
vlinder agent register ./agent.toml --code ./agent.wasm
→ Registered: registry://localhost:9000/agents/pensieve
```

### Model Manifest Changes

Remove `id` field (physical location). The manifest describes what the model *is*, not where it *lives*.

See: [038-examples/current-model-manifest.toml](038-examples/current-model-manifest.toml)
See: [038-examples/proposed-model-manifest.toml](038-examples/proposed-model-manifest.toml)

| Field | Current | Proposed |
|-------|---------|----------|
| `name` | `"phi3"` | `"phi3"` (unchanged) |
| `type` | `"inference"` | `"inference"` (unchanged) |
| `engine` | `"llama"` | `"llama"` (unchanged) |
| `id` | `"file:///path/to/weights.gguf"` | Removed — registry assigns ID, tracks artifact location |

### Agent Manifest Changes

Remove `id` field and inline storage URIs. Model references become names, not paths.

See: [038-examples/current-agent-manifest.toml](038-examples/current-agent-manifest.toml)
See: [038-examples/proposed-agent-manifest.toml](038-examples/proposed-agent-manifest.toml)

| Field | Current | Proposed |
|-------|---------|----------|
| `name` | `"pensieve"` | `"pensieve"` (unchanged) |
| `id` | `"agent.wasm"` | Removed — registry assigns ID, tracks artifact location |
| `requirements.models` | `{ phi3 = "file://..." }` | `["phi3", "nomic-embed"]` (names only) |
| `object_storage` | `"sqlite:///path"` | Removed |
| `vector_storage` | `"sqlite:///path"` | Removed |
| `requirements.storage` | (missing) | `{ object = true, vector = true }` |

### Registry Responsibilities

The registry becomes the authority for:

1. **Identity assignment**: Issues `registry://` URIs for all resources
2. **Artifact tracking**: Maps logical IDs to physical locations
3. **Name resolution**: Resolves model names in agent requirements to model IDs
4. **Storage provisioning**: Provides storage to agents that declare the requirement

### Resolution at Runtime

When an agent is invoked:

1. Harness looks up agent by name → gets `registry://localhost:9000/agents/pensieve`
2. Registry returns agent metadata including:
   - Code artifact location
   - Required models (resolved to their IDs)
   - Storage configuration (provisioned by registry)
3. Runtime loads code and provisions dependencies

## Consequences

### Positive

- **Portable manifests**: Same manifest works anywhere; physical location is separate
- **Model as domain entity**: Models are registered, cataloged, shareable by name
- **Registry-centric**: Single source of truth for resource identity and location
- **Clean separation**: Manifest = what it is; Registry = where it lives

### Negative

- **Registration step required**: Can't just point at files anymore; must register first
- **Registry dependency**: Need a running registry to resolve resources

## Open Questions

- How does storage get provisioned and bound to agents?
- How are mounts handled? (They reference host paths, inherently physical)
- What's the workflow for local development without registry ceremony?

## See Also

- ADR 023: Model Manifest Format (current schema)
- ADR 031: vlinderd as Runtime Registry (registry concept)
