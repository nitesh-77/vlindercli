# ADR 110: Feature-Gated Provider Crates

**Status:** Draft

## Context

The workspace has 18 crates. Every binary pulls in every provider regardless
of what it actually needs. The four provider crates (vlinder-ollama,
vlinder-infer-openrouter, vlinder-sqlite-kv, vlinder-sqlite-vec) are
unconditional dependencies in three places:

1. **vlinderd** — uses them to spawn worker processes
2. **vlinder-provider-server** — uses them to build virtual-host routing tables
3. **vlinder-podman-runtime** — uses them only for HOSTNAME constants

The sidecar and lambda adapter inherit all four providers transitively through
vlinder-provider-server. vlinderd also pulls in vlinder-podman-runtime (which
pulls in providers again) and vlinder-nats-lambda-runtime (AWS SDK). The
result: every binary contains everything.

### Current state

```
vlinderd (binary)
├── vlinder-ollama
├── vlinder-infer-openrouter
├── vlinder-sqlite-kv
├── vlinder-sqlite-vec
├── vlinder-podman-runtime
│   ├── vlinder-ollama          ← only for HOSTNAME constant
│   ├── vlinder-infer-openrouter ← only for HOSTNAME constant
│   ├── vlinder-sqlite-kv       ← only for HOSTNAME constant
│   └── vlinder-sqlite-vec      ← only for HOSTNAME constant
├── vlinder-nats-lambda-runtime  ← AWS SDK (not needed for container-only)
├── vlinder-nats
├── vlinder-sql-registry
├── vlinder-sql-state
├── vlinder-git-dag
├── vlinder-harness
└── vlinder-catalog

vlinder-podman-sidecar (binary)
└── vlinder-provider-server
    ├── vlinder-ollama
    ├── vlinder-infer-openrouter
    ├── vlinder-sqlite-kv
    ├── vlinder-sqlite-vec
    ├── vlinder-nats
    ├── vlinder-sql-registry
    └── vlinder-sql-state

vlinder-lambda-adapter (binary)
└── vlinder-provider-server
    ├── vlinder-ollama
    ├── vlinder-infer-openrouter
    ├── vlinder-sqlite-kv
    ├── vlinder-sqlite-vec
    ├── vlinder-nats
    ├── vlinder-sql-registry
    └── vlinder-sql-state
```

Problems:
- Sidecar binary includes all providers even if the agent only uses inference
- Lambda adapter includes all providers even if the agent only uses KV
- vlinderd includes AWS SDK even for container-only deployments
- vlinder-podman-runtime depends on 4 provider crates for 4 string constants
- Compile times scale with the union of all dependencies

## Decision

Feature-gate provider crates so consumers opt in to what they need.
Extract one crate at a time — each step compiles, tests, and commits
independently.

### Step 0: Move HOSTNAME constants to vlinder-core

The four provider crates each export a `HOSTNAME: &str` constant.
vlinder-podman-runtime depends on all four just for these constants.
Move them to vlinder-core and remove the provider deps from
vlinder-podman-runtime.

### Step 1: Feature-gate vlinder-provider-server

Make each provider an optional dependency behind a Cargo feature.
Default features include all four. `build_hosts()` uses
`#[cfg(feature = "...")]` to conditionally include provider hosts.

Consumers inherit defaults. Slim builds opt out:
```toml
vlinder-provider-server = { default-features = false, features = ["ollama"] }
```

### Step 2: Feature-gate vlinderd

Same pattern for worker functions. Each provider worker gets
`#[cfg(feature)]` guards. Lambda and container runtimes also become
features so container-only deployments don't pull in AWS SDK.

### Desired state

```
vlinderd (binary) — features: ollama, openrouter, sqlite-kv, sqlite-vec, lambda, container
├── vlinder-ollama              (optional, feature = "ollama")
├── vlinder-infer-openrouter    (optional, feature = "openrouter")
├── vlinder-sqlite-kv           (optional, feature = "sqlite-kv")
├── vlinder-sqlite-vec          (optional, feature = "sqlite-vec")
├── vlinder-podman-runtime      (optional, feature = "container")
│   └── vlinder-core            ← HOSTNAME constants from core, no provider deps
├── vlinder-nats-lambda-runtime (optional, feature = "lambda")
├── vlinder-nats
├── vlinder-sql-registry
├── vlinder-sql-state
├── vlinder-git-dag
├── vlinder-harness
└── vlinder-catalog

vlinder-podman-sidecar (binary) — features: ollama, openrouter, sqlite-kv, sqlite-vec
└── vlinder-provider-server
    ├── vlinder-ollama              (optional)
    ├── vlinder-infer-openrouter    (optional)
    ├── vlinder-sqlite-kv           (optional)
    ├── vlinder-sqlite-vec          (optional)
    ├── vlinder-nats
    ├── vlinder-sql-registry
    └── vlinder-sql-state

vlinder-lambda-adapter (binary) — features: ollama, openrouter, sqlite-kv, sqlite-vec
└── vlinder-provider-server
    ├── (same optional providers)
    ...
```

Default features = everything. Existing builds don't change. Slim builds
disable what they don't need. CI can produce per-target binaries:
- `cargo build -p vlinderd --features container,ollama,sqlite-kv` (no Lambda, no OpenRouter)
- `cargo build -p vlinder-lambda-adapter --no-default-features --features ollama` (minimal Lambda)

## Consequences

- Each step is independently committable — no big-bang refactor
- Default features preserve current behavior — nothing breaks
- vlinder-podman-runtime loses 4 unnecessary dependencies
- Slim binaries become possible for Lambda, container-only, or
  provider-specific deployments
- Compile times improve for non-default builds
- CI can produce multiple binary variants per platform (connects to #32)
