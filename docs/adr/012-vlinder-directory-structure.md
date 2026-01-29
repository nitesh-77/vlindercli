# ADR 012: Vlinder Directory Structure

## Status

Superseded by ADR 020

ADR 020 replaced the global `.vlinder/agents/` registry with self-contained agent directories. Agents are now loaded by path (`Agent::load(path)`), not by name from a central location.

## Context

Vlinder needs a consistent location for:
- Agent wasm files
- Agent storage databases
- Model files

Previously, these were scattered: `agent_wasm/`, `models/`, `.agentfs/`. This made agents feel fragmented rather than self-contained.

The vision describes agents as portable packages (Vlinderbox) containing wasm, storage, and manifest together.

## Decision

**Single `.vlinder` directory with convention-based paths.**

```
.vlinder/
├── agents/
│   ├── reader-agent/
│   │   ├── reader_agent.wasm    # agent code
│   │   └── reader-agent.db      # agent storage (AgentFS)
│   └── echo-agent/
│       ├── echo_agent.wasm
│       └── echo-agent.db
└── models/
    ├── phi3.gguf
    └── deepseek.gguf
```

**Path resolution via config module:**

```rust
// src/config.rs
pub fn vlinder_dir() -> PathBuf { ... }
pub fn agent_wasm_path(name: &str) -> PathBuf { ... }
pub fn agent_db_path(name: &str) -> PathBuf { ... }
pub fn model_path(model_name: &str) -> PathBuf { ... }
```

**Config layering (future-ready):**

```
CLI args > env var (VLINDER_DIR) > config file > default (.vlinder)
```

Currently only default is implemented. Layers can be added incrementally.

## Consequences

- Agents are self-contained: wasm + storage live together
- Single directory to backup, move, or gitignore
- Path logic centralized in `config.rs`
- Ready for user-level directory when we move out of repo
- Future: config file at platform-appropriate location (via `dirs` crate)
