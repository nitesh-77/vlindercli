# ADR 110: Feature-Gated Crate Dependencies

**Status:** Draft

## Context

The workspace has 18 crates and 4 binaries. Two dependency problems exist:

### Problem 1: Provider crates are unconditional

The four provider crates (vlinder-ollama, vlinder-infer-openrouter,
vlinder-sqlite-kv, vlinder-sqlite-vec) are unconditional dependencies in:

1. **vlinderd** — uses them to spawn worker processes
2. **vlinder-provider-server** — uses them to build virtual-host routing tables
3. **vlinder-podman-runtime** — uses them only for HOSTNAME constants

The sidecar and lambda adapter inherit all four providers transitively through
vlinder-provider-server. vlinderd also pulls in vlinder-podman-runtime (which
pulls in providers again) and vlinder-nats-lambda-runtime (AWS SDK). The
result: every binary contains every provider.

### Problem 2: Service crates bundle client + server

Five service crates each bundle gRPC client, gRPC server, and storage
implementations in a single crate:

| Crate | CLI needs | Also pulls in |
|-------|-----------|---------------|
| vlinder-harness | `GrpcHarnessClient` | `HarnessServiceServer` |
| vlinder-sql-registry | `GrpcRegistryClient` | `RegistryServiceServer` + `SqliteRegistryRepository` + `rusqlite` |
| vlinder-sql-state | `GrpcStateClient` | `StateServiceServer` + `SqliteDagStore` + `SessionServer` + `tiny_http` + `rusqlite` |
| vlinder-nats | `GrpcSecretClient` | `SecretServiceServer` + `NatsQueue` + `NatsSecretStore` + `async-nats` |
| vlinder-catalog | `GrpcCatalogClient` | `CatalogServiceServer` |

The vlinder CLI binary only needs gRPC clients (thin protobuf stubs) but
drags in SQLite (bundled), NATS runtime, tiny_http, and all server
implementations.

### Current state

```
vlinder (CLI binary)
├── vlinder-core
├── vlinder-harness           ← only needs client, gets server too
├── vlinder-sql-registry      ← only needs client, gets server + rusqlite
├── vlinder-sql-state         ← only needs client, gets server + rusqlite + tiny_http
├── vlinder-nats              ← only needs client, gets server + async-nats
└── vlinder-catalog           ← only needs client, gets server too

vlinderd (daemon binary)
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
- CLI binary includes SQLite, NATS runtime, tiny_http — needs none of them
- Sidecar includes all providers even if the agent only uses inference
- Lambda adapter includes all providers even if the agent only uses KV
- vlinderd includes AWS SDK even for container-only deployments
- vlinder-podman-runtime depends on 4 provider crates for 4 string constants
- Compile times scale with the union of all dependencies

## Decision

Feature-gate dependencies within existing crates. No crate splits — client
and server code stay co-located next to the proto definition. Each step
compiles, tests, and commits independently.

### Track A: Client/server feature gates (service crates)

Add `client` and `server` features to each service crate. Heavy dependencies
(rusqlite, tiny_http, async-nats) move behind the `server` feature. The proto
module and client stay behind the `client` feature.

Pattern for each service crate:

```toml
[features]
default = ["client", "server"]
client = []
server = ["dep:rusqlite"]  # heavy deps only needed by server
```

The CLI opts into just `features = ["client"]`. The daemon gets defaults.

Crates to feature-gate:
1. **vlinder-harness** — client/server
2. **vlinder-sql-registry** — client/server (rusqlite behind server)
3. **vlinder-sql-state** — client/server (rusqlite + tiny_http behind server)
4. **vlinder-nats** — client/server (async-nats behind server)
5. **vlinder-catalog** — client/server

### Track B: Provider feature gates

Feature-gate provider crates in their consumers so builds opt in to what
they need.

Crates to feature-gate:
1. **vlinder-podman-runtime** — ollama, openrouter, sqlite-kv, sqlite-vec
2. **vlinder-provider-server** — ollama, openrouter, sqlite-kv, sqlite-vec
3. **vlinderd** — providers + lambda/container runtime selection

### Desired state

```
vlinder (CLI binary)
├── vlinder-core
├── vlinder-harness           { features = ["client"] }
├── vlinder-sql-registry      { features = ["client"] }
├── vlinder-sql-state         { features = ["client"] }
├── vlinder-nats              { features = ["client"] }
└── vlinder-catalog           { features = ["client"] }
  → No rusqlite, no tiny_http, no async-nats, no server code

vlinderd (daemon binary) — features: ollama, openrouter, sqlite-kv, sqlite-vec, lambda, container
├── vlinder-ollama              (optional, feature = "ollama")
├── vlinder-infer-openrouter    (optional, feature = "openrouter")
├── vlinder-sqlite-kv           (optional, feature = "sqlite-kv")
├── vlinder-sqlite-vec          (optional, feature = "sqlite-vec")
├── vlinder-podman-runtime      (optional, feature = "container")
├── vlinder-nats-lambda-runtime (optional, feature = "lambda")
├── vlinder-nats                { default-features }
├── vlinder-sql-registry        { default-features }
├── vlinder-sql-state           { default-features }
├── vlinder-git-dag
├── vlinder-harness             { default-features }
└── vlinder-catalog             { default-features }

vlinder-podman-sidecar (binary)
└── vlinder-provider-server
    ├── vlinder-ollama              (optional)
    ├── vlinder-infer-openrouter    (optional)
    ├── vlinder-sqlite-kv           (optional)
    ├── vlinder-sqlite-vec          (optional)
    ...

vlinder-lambda-adapter (binary)
└── vlinder-provider-server
    ├── (same optional providers)
    ...
```

Default features = everything. Existing builds don't change. Slim builds
disable what they don't need:
- `cargo build -p vlinder` — CLI with client-only deps (no SQLite, no NATS)
- `cargo build -p vlinderd --features container,ollama` — no Lambda, no OpenRouter
- `cargo build -p vlinder-lambda-adapter --no-default-features --features ollama` — minimal Lambda

## Consequences

- Each step is independently committable — no big-bang refactor
- Default features preserve current behavior — nothing breaks
- CLI binary drops rusqlite, tiny_http, async-nats, and all server code
- Slim daemon/sidecar/adapter binaries become possible
- Client and server code stay co-located — no impedance mismatch from splits
- Compile times improve for non-default builds
- CI can produce multiple binary variants per platform (connects to #32)
