# ADR 102: Content-Addressed Agent Manifest

**Status:** Draft

## Context

When an agent is deployed, the user submits an `AgentManifest` (the
hydrated, fully-resolved manifest from `agent.toml`). Today this
manifest is consumed during `Agent::from_manifest()` and discarded —
the registry only stores the resolved `Agent`.

This creates two problems:

1. **Idempotency requires hand-rolled comparison.** The harness
   currently compares agents field-by-field to detect re-deploys
   (`compare_agents()` in `harness.rs`). The `Agent` struct contains
   registry-assigned fields (`id`, `public_key`, `image_digest`) that
   differ between a fresh agent and a stored one, so naive `==` fails.

2. **No audit trail.** There is no record of what was deployed, only
   the resolved result. You cannot answer "what changed between
   deploys?" or roll back to a previous configuration.

## Decision

Agent manifests are **immutable artifacts** with content-addressed
identity. The registry stores manifests and treats them the same way
a container registry treats images.

### Identity

The manifest ID is a SHA-256 hash of its serialized content — the same
content-addressing scheme used for submissions (ADR 044) and state
(ADR 055). Two identical manifests produce the same ID regardless of
when or where they were deployed.

### Immutability

Once stored, a manifest never changes. A new deployment with different
configuration produces a new manifest with a new content hash. The old
manifest remains in the store.

### Agent names as tags

Agent names follow the container image convention: `name:tag`
(e.g., `todoapp:latest`, `todoapp:v1`). A name is a tag pointing to
an immutable manifest.

The existing `ImagePolicy` (pinned vs unpinned) generalizes beyond
container images to the entire deployment model:

- **Unpinned** (e.g., `:latest`) — tag can be moved to a new manifest
- **Pinned** (e.g., `:v1`) — tag is locked to its manifest

### Registry API

`register_agent()` accepts an `AgentManifest` and returns the resolved
`Agent`:

```rust
fn register_agent(&self, manifest: AgentManifest) -> Result<Agent, RegistrationError>;
```

The registry owns the full lifecycle: validate → resolve → assign
identity → store manifest + agent → return agent.

### Idempotency

Idempotency follows naturally from content-addressing:

- **Same content hash** → manifest already stored, return existing agent
- **Different content hash** → new manifest (tag behavior depends on
  pin policy; for now, treated as an error until tagging is implemented)

No hand-rolled field comparison. `AgentManifest` and its sub-types
derive `PartialEq`.

### Storage

The registry stores a mapping: `agent_name → (manifest, agent)`. The
`Agent` struct does not grow — it references the manifest by name.
The manifest is persisted in `StoredAgent` / `SqliteRegistryRepository`.

## Consequences

- `AgentManifest`, `RequirementsConfig`, `PromptsConfig`, `MountConfig`
  derive `PartialEq`.
- Manifests are immutable — cheap storage, rich audit trail.
- Idempotency moves from the harness into the registry (ADR 101).
- The `compare_agents()` function in `harness.rs` becomes dead code.
- `ImagePolicy` becomes a platform-wide concept, not just for Podman.
- Future: `name:tag` versioning, deploy history, rollback, diff.
