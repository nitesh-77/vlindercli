# ADR 027: Manifest Dependencies (Ports/Adapters Enforcement)

## Status

Draft

## Context

Good agent architecture follows ports and adapters / hexagonal architecture: core logic is isolated, external dependencies are explicit at the boundaries. The agent manifest already declares some dependencies (models, services, mounts). This ADR extends that to include agent-to-agent dependencies.

Declaring dependencies explicitly enables:
- Auditing: what can this agent access?
- Static analysis: build dependency graphs before deployment
- Runtime enforcement: reject undeclared calls

## Decision

### 1. Agents Declare Agent Dependencies

The manifest `requirements` section includes an `agents` list declaring which other agents this agent may call.

```toml
[requirements]
models = { phi3 = "file://./models/phi3.gguf" }
services = ["infer", "embed", "store"]
agents = ["chunker", "summarizer", "embedder"]  # NEW
```

**Rationale:**
- Makes agent's external interface visible in one place
- Enables dependency graph analysis
- Enforces ports/adapters pattern—no hidden dependencies

### 2. Runtime Validation

The SDK validates calls against the manifest at runtime. Calling an undeclared agent returns an error.

```rust
// Manifest declares: agents = ["chunker"]

// This works
sdk.call_agent("chunker", input, Timeout::Seconds(30))?;

// This fails immediately
sdk.call_agent("unknown", input, Timeout::Seconds(30))?;
// => Err(SdkError::UndeclaredDependency { agent: "unknown" })
```

**Why not compile-time?**
- Agents are distributed; target agent may not exist at compile time
- Agent names are strings resolved at runtime
- Compile-time checking is not possible in this architecture

**Why not documentation-only?**
- No enforcement means the declaration drifts from reality
- Defeats the purpose of explicit dependencies

### 3. Explicit List Only (No Wildcards)

Agent dependencies must be explicitly enumerated. No wildcards or patterns.

```toml
# Good - explicit
agents = ["chunker", "summarizer", "embedder"]

# Not supported - no wildcards
agents = ["*"]
agents = ["fleet:*"]
agents = ["summarizer-*"]
```

**Rationale:**
- Wildcards defeat auditability
- If an agent needs to call dynamic/unknown agents, that's a code smell
- Explicit lists enable static dependency analysis

## Consequences

- **Auditable:** Manifest is the source of truth for what an agent can access
- **Enforceable:** Runtime validation catches configuration errors early
- **Static analysis:** Can build agent dependency graphs from manifests alone
- **No hidden dependencies:** All external access is visible

## Open Questions

- Should environment variables be declared in manifest?
- Should external API endpoints be declared?
- How does this interact with fleet-level configuration?
