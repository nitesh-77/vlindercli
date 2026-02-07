# ADR 049: Persistent Registry

## Status

Proposed

## Context

Model persistence has three interconnected problems:

1. **Silent error swallowing** — `Daemon::load_registered_models()` and the
   registry worker silently return on any SQLite failure. No logging, no error.
   A corrupt or missing database looks identical to "no models registered."

2. **No write-through** — `register_model()` on InMemoryRegistry only writes
   to the in-memory HashMap. Models added via `vlinder model add` go to SQLite
   but never touch the registry. Models loaded at daemon startup go to memory
   but never touch SQLite. Neither path updates both stores.

3. **Two disconnected paths** — the CLI writes directly to SqliteRegistryRepository.
   The daemon/worker loads from SqliteRegistryRepository into InMemoryRegistry.
   These are independent code paths with no shared abstraction.

Design principles: fail-fast with clear errors, SQLite as source of truth,
in-memory as cache, one system and one way to add models.

## Decision

Create `PersistentRegistry` — a write-through composition of
`InMemoryRegistry` + `SqliteRegistryRepository`.

### Signature change

`register_model(&self, model: Model)` becomes
`register_model(&self, model: Model) -> Result<(), RegistrationError>`.

New `RegistrationError::Persistence(String)` variant for storage failures.
This also fixes GrpcRegistryClient which was silently discarding network
errors via `let _ = ...`.

### PersistentRegistry behavior

- **Constructor** (`open`): opens SQLite, loads all models into
  InMemoryRegistry. Fails fast with a clear error on any failure.
- **register_model**: writes to SQLite first (fail-fast), then updates
  InMemoryRegistry cache.
- **Reads**: delegate to InMemoryRegistry (fast, no I/O).
- **delete_model**: removes from SQLite first, then removes from cache.
  This method lives on PersistentRegistry directly, not on the Registry trait.
- **All other methods**: delegate to InMemoryRegistry unchanged.

### Unified path

All model operations flow through PersistentRegistry:

- `vlinder model add` → `PersistentRegistry::register_model()`
- `vlinder model remove` → `PersistentRegistry::delete_model()`
- `vlinder model registered` → `PersistentRegistry::get_models()`
- Daemon startup → `PersistentRegistry::open()` loads models
- Registry worker → `PersistentRegistry::open()` loads models
- gRPC server → wraps PersistentRegistry (distributed clients persist
  through the server)

`load_registered_models()` is eliminated — the constructor handles loading.

### What stays unchanged

- `InMemoryRegistry` — still useful for tests, still the inner implementation
- `RegistryRepository` trait — unchanged
- `SqliteRegistryRepository` — unchanged, used internally by PersistentRegistry
- gRPC proto — only additive change (optional error field on RegisterModelResponse)

## Consequences

- One path for all model operations — disk and memory always in sync
- Fail-fast errors with clear messages replace silent swallowing
- CLI commands no longer bypass the registry abstraction
- Tests continue using InMemoryRegistry directly (no SQLite in unit tests)
- Distributed mode gets persistence automatically: gRPC server wraps
  PersistentRegistry, so remote `register_model` calls persist server-side
