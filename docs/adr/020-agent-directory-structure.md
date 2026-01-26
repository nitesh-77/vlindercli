# ADR 020: Agent Directory Structure

## Status

Accepted

## Context

Agents are currently loaded by name from `.vlinder/agents/<name>/`. This couples agent identity to a global registry location.

We want agents to be self-contained and runnable from any location:
```bash
cd path/to/my-agent/
vlinder run
```

## Decision

**An agent directory is self-contained and runnable.**

```
my-agent/
├── agent.toml          # manifest (required)
├── agent.wasm          # compiled agent (required)
├── agent.db            # runtime storage (gitignored)
└── mnt/                # optional mount directory
```

**Conventional naming** — like `Cargo.toml` in a Rust crate:
- Manifest is always `agent.toml`
- WASM binary path is declared via `code` field (required — an agent must have code to run)

**CLI runs from agent directory:**

```bash
cd my-agent/
vlinder run                      # uses cwd
vlinder run -p /path/to/agent    # explicit path
```

**Agent loading takes a path:**

```rust
Agent::load(path)  // looks for path/agent.toml
```

The manifest contains a `name` field for display/logging. Directory name matching manifest name is convention, not enforced.

## Consequences

- Agents are fully self-contained and portable
- Can run agent from any directory
- Path-based loading instead of name-based registry
- No dependency on global `.vlinder/` directory structure
- Simplifies testing — fixtures are just directories with agent.toml + agent.wasm
