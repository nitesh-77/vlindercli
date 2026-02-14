# ADR 039: Bindings as Domain Entity

## Status

Superseded — registry handles capability binding directly (ADR 035, 048)

## Context

ADR 038 separates logical identity from physical location. The registry assigns identity and tracks artifact locations. But this raises a question: how do we model the relationship between a logical entity and its physical realization in a given environment?

Consider:
- An agent `pensieve` needs model `phi3` and storage
- In dev, `phi3` → local GGUF file, storage → SQLite
- In prod, `phi3` → S3-hosted weights, storage → Pinecone

The *agent* is the same logical entity. The *binding* differs per environment.

## The Concept

A **binding** connects a logical entity to its physical realization:

```
Logical Entity (what)     Binding (where, in this context)
─────────────────────     ────────────────────────────────
model "phi3"         →    file:///local/phi3.gguf
agent "pensieve"     →    file:///local/agent.wasm
storage requirement  →    sqlite:///local/data.db
```

Bindings could be:
- Per-agent (each agent has its own binding)
- Per-environment (dev, staging, prod)
- Per-fleet (all agents in a fleet share bindings)

## Open Questions

These need discussion before this ADR can progress:

### 1. Terminology Clarity

Current terms and their meanings:

| Term | Current Understanding |
|------|----------------------|
| **Manifest** | TOML file describing an entity (agent.toml, model.toml) |
| **Entity** | The domain object (Agent, Model) created from a manifest |
| **Binding** | Maps logical requirements to physical resources |

Is this the right vocabulary? Alternatives:
- Manifest vs Specification vs Definition?
- Binding vs Resolution vs Realization?
- Entity vs Resource vs Artifact?

### 2. Where Do Bindings Live?

Options:

**A. Separate binding files** (like Terraform tfvars)
```
.vlinder/bindings/
├── dev.toml
├── staging.toml
└── prod.toml
```

**B. Fleet manifest** (deployment = fleet + environment)
```toml
# fleet.toml
[environments.dev]
models.phi3 = "file:///local/phi3.gguf"

[environments.prod]
models.phi3 = "s3://models/phi3.gguf"
```

**C. Registry manages bindings** (binding is just registry state)
```
vlinder bind phi3 --to file:///local/phi3.gguf --env dev
```

**D. Implicit from registration** (no separate binding concept)
```
# Registration IS the binding
vlinder model register phi3.toml --weights ./phi3.gguf
```

### 3. Binding Granularity

What gets bound?

| Resource | Needs Binding? |
|----------|---------------|
| Model weights | Yes — different files per environment |
| Agent code | Yes — different artifacts per environment |
| Storage | Yes — SQLite vs S3 vs Pinecone |
| Mounts | Yes — host paths are physical |
| Services | Maybe — service endpoints could vary |

### 4. Binding Lifecycle

- When is a binding created?
- Can bindings be updated? (swap model weights without re-registering?)
- Are bindings versioned independently of the entity?

### 5. Relationship to Registry

If registry owns identity (ADR 038), does it also own bindings?

```
Registry stores:
  - Entity identity: registry://localhost:9000/models/phi3
  - Entity metadata: { name, type, engine }
  - Binding: file:///var/vlinder/models/phi3.gguf  ← is this part of the entity or separate?
```

Or is a binding a separate concept that references entities?

```
Binding {
  entity: registry://localhost:9000/models/phi3
  artifact: file:///var/vlinder/models/phi3.gguf
  environment: "dev"
}
```

## Parking This

The core insight — that logical entities need physical realization, and this mapping is its own concern — seems sound. But the modeling needs more thought.

Specifically:
- Don't want to over-engineer before we have real multi-environment needs
- Don't want to conflate registration with binding if they're separate concerns
- Need to understand the local dev workflow before adding ceremony

## See Also

- ADR 038: Logical vs Physical Manifest Schema
- ADR 031: vlinderd as Runtime Registry
