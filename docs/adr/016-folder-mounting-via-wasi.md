# ADR 016: Folder Mounting via WASI

## Status

Accepted (extends ADR 013)

## Context

ADR 013 established SQLite-based storage for agent persistence. This works well for structured data, embeddings, and cached content. However, some use cases require access to real host directories:

1. Processing local files (PDFs, documents, images)
2. Exporting outputs to user-accessible locations
3. Working with existing folder structures without importing into SQLite

We need a way for agents to access host filesystem directories while maintaining security boundaries.

## Decision

**Declare filesystem mounts in agent manifest, expose via WASI preopened directories.**

```toml
[[mounts]]
host_path = "mnt"                              # Relative to agent dir
guest_path = "/"
mode = "rw"

[[mounts]]
host_path = "/Users/me/Documents/articles"     # Absolute path
guest_path = "/articles"
mode = "ro"
```

**Path resolution:**
- Relative `host_path`: resolved against `.vlinder/agents/<name>/`
- Absolute `host_path`: used as-is
- `guest_path`: must start with `/`
- `mode`: `"ro"` or `"rw"`, defaults to `"rw"`

**Default mount:** If no `[[mounts]]` declared, auto-add `mnt/` → `/` (rw)

**Implementation via Extism's WASI support:**

```rust
let mut manifest = Manifest::new([wasm]).with_allowed_host("*");

for (host_path, guest_path) in agent.resolve_mounts() {
    manifest = manifest.with_allowed_path(host_path, guest_path);
}

// Read-only enforced via "ro:" prefix convention
let key = if mode == "ro" {
    format!("ro:{}", host_path.display())
} else {
    host_path.display().to_string()
};
```

**Agents use standard filesystem APIs:**

```rust
// In agent WASM code (compiled for wasm32-wasip1)
use std::fs;

let content = fs::read_to_string("/articles/paper.txt")?;
fs::write("/output.txt", "result")?;
```

## Consequences

- Agents can access real host directories without custom host functions
- WASI preopened directories prevent path traversal attacks
- Read-only enforcement at WASI layer, not application layer
- Coexists with SQLite storage — choose based on data type:
  - **SQLite**: structured data, embeddings, caching, portability
  - **WASI mounts**: real files, exports, existing folder structures
- Non-existent mount paths are skipped with warning (graceful degradation)
- Runtime auto-creates default `mnt/` directory on first execution
