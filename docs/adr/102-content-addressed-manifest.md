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

The `AgentManifest` is a first-class entity with a content-addressed
identity. The registry stores manifests alongside agents.

### Identity

The manifest ID is a SHA-256 hash of its serialized content — the same
content-addressing scheme used for submissions (ADR 044) and state
(ADR 055). Two identical manifests produce the same ID regardless of
when or where they were deployed.

### Storage

The registry stores a mapping: `agent_name → manifest`. The `Agent`
struct does not grow — it references the manifest by name (which is
already a field). The manifest is persisted in `StoredAgent` /
`SqliteRegistryRepository`.

### Idempotency

`register_agent()` becomes idempotent at the registry level:

- Look up existing agent by name
- If found, compare incoming manifest with stored manifest (`==`)
- Same manifest → `Ok(())` (no-op)
- Different manifest → `ConfigMismatch` error

No hand-rolled field comparison. `AgentManifest` and its sub-types
derive `PartialEq`.

### Deploy history

Content-addressed manifests enable future deploy history: a log of
`(timestamp, manifest_hash)` per agent. This is deferred — the
immediate goal is idempotency and audit trail for the current deploy.

## Consequences

- `AgentManifest`, `RequirementsConfig`, `PromptsConfig`, `MountConfig`
  derive `PartialEq`.
- The registry stores the manifest — cheap storage, rich audit trail.
- Idempotency moves from the harness into the registry (ADR 101).
- The `compare_agents()` function in `harness.rs` becomes dead code.
- Future: deploy history, rollback, diff across deploys.
