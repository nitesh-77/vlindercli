# ADR 045: Daemon / Supervisor Split

## Status

Accepted

## Context

The `Daemon` struct serves two unrelated purposes depending on config:

**Local mode:** Owns all domain components (registry, harness, runtime, provider) and ticks them in a loop. This is correct — it's the single-process development experience.

**Distributed mode:** Spawns child worker processes, then sits in a tick loop doing almost nothing. It creates a `CliHarness` and `InMemoryRegistry` that are never used (the CLI process creates its own harness with gRPC registry). The `shutdown` field is never read. `is_distributed()` checks `!self.workers.is_empty()`, silently degrading to local mode if all spawns fail.

The struct uses `Option<Runtime>`, `Option<Provider>`, and conditional logic to handle both modes. These modes are mutually exclusive — you never spawn processes in local mode, and you never own a runtime in distributed mode.

### Problems

1. **Zombie fields** — distributed mode creates harness + registry it never uses
2. **Dead code** — `shutdown` field written but never read
3. **Fragile detection** — `is_distributed()` masks spawn failures
4. **God object** — couples domain orchestration with process supervision
5. **Duplication** — capability registration appears in `Daemon::new_local`, `Daemon::new_distributed`, and `worker.rs::run_registry_worker`

## Decision

Split the daemon into two types that reflect the mutual exclusion:

### 1. `Daemon` — local mode only

Owns all domain components. Unchanged from current local-mode behavior:

```rust
pub struct Daemon {
    registry: Arc<dyn Registry>,
    pub harness: CliHarness,
    runtime: WasmRuntime,
    provider: Provider,
}
```

No `Option` fields. No `workers`. No `shutdown`. No `is_distributed()`. Created by `vlinder agent run` for the embedded development experience.

### 2. `Supervisor` — distributed process manager

Owns child processes. No domain objects:

```rust
pub struct Supervisor {
    workers: Vec<Child>,
}
```

Responsibilities:
- Spawn workers from config (registry first, then others)
- Terminate workers on shutdown

No harness, no registry, no queue. The supervisor doesn't participate in the message flow — it just manages worker lifecycle.

### 3. Command routing

```
vlinder daemon          → Supervisor (distributed) or Daemon tick loop (local)
vlinder agent run       → Embedded Daemon (always local)
vlinder worker          → Standalone worker process (for manual/advanced use)
```

## Consequences

**Positive:**
- No more zombie fields or dead code
- Type system enforces mutual exclusion (not runtime `Option` checks)
- Supervisor can add health checks and restart without touching domain code

**Negative:**
- Two types to understand instead of one (but each is simpler)

## Deferred

- Consolidate capability registration (still duplicated in `Daemon::new` and `run_registry_worker`)
- Generic worker loop (six copy-pasted worker functions remain)
- Registry health check (still uses 500ms sleep hack)
