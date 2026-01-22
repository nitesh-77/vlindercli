# ADR 017: Agent Manifest Naming Convention

## Status

Accepted (supersedes ADR 015)

## Context

ADR 015 established the agent manifest concept but used `Vlinderfile` as the filename. This had drawbacks:

1. Generic name doesn't identify which agent it belongs to
2. Follows Docker/Vagrant convention, but those are single-file projects
3. Harder to work with multiple manifests (copying, diffing, grepping)

The manifest should be the primary artifact that gets source-controlled. Everything else (wasm binary, database, mnt directory) is derived or runtime-generated.

## Decision

**Agent manifest is `<name>-agent.toml`.**

```
.vlinder/agents/pensieve/
├── pensieve-agent.toml    # Source controlled - the agent, serialized
├── pensieve.wasm          # Built or fetched by runtime
├── pensieve.db            # Created by runtime
└── mnt/                   # Created by runtime
```

The manifest file is self-identifying. You can:
- Copy it somewhere and know what it is
- Grep across manifests easily
- Source control just the manifests

**Runtime responsibilities:**
- Fetch or build wasm if missing
- Create database on first run
- Create mnt directory on first run

**Host function renamed:**
```rust
// Old
pub fn get_vlinderfile() -> String;

// New
pub fn get_manifest() -> String;
```

## Consequences

- Manifest is the artifact you commit; everything else is derived
- Agent directories can be bootstrapped from manifest alone
- Clearer naming: "manifest" over branded "Vlinderfile"
- Breaking change to host function name (`get_manifest` vs `get_vlinderfile`)
