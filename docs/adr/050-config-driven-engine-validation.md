# ADR 050: Config-Driven Engine Validation

**Status:** Accepted

## Context

The daemon hardcoded all engine types as available regardless of configuration. An agent requiring OpenRouter could be deployed even when no OpenRouter workers were configured, leading to silent failures at invocation time (requests hang with no worker to handle them).

Similarly, `model add` accepted any model without checking whether the system could actually serve it.

## Decision

Engine availability is validated at every boundary, driven by config worker counts:

1. **Model registration** — `register_model()` rejects models whose engine isn't configured (e.g. OpenRouter model when `workers.inference.openrouter = 0`).
2. **Startup** — `PersistentRegistry::open(db_path, config)` registers engines from config before loading persisted models. If config changes remove an engine, startup fails fast instead of silently loading unservable models.
3. **Agent deployment** — `register_agent()` validates that each model's engine is available (defense in depth).

`PersistentRegistry::open()` takes `&Config` to guarantee correct initialization order: engines registered before models loaded.

## Consequences

- `model add` with an unconfigured engine fails immediately with a clear error.
- Changing config to remove an engine while models exist in the db causes a startup error — operator must remove the models first or restore the config.
- Engine registration logic lives in one place (`PersistentRegistry::open`) instead of being duplicated in daemon and worker.
