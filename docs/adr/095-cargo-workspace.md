# ADR 095: Cargo Workspace Split

## Status

Draft

## Context

The codebase is a single crate (`vlindercli`) that produces one binary.
Everything — domain types, queue implementations, SQLite storage, gRPC
servers, inference engines, Podman runtime, CLI harness — lives in one
`Cargo.toml` with one dependency list.

This causes two problems:

1. **Every binary links against everything.** Any future binary target
   (like the sidecar from ADR 091) would inherit all dependencies —
   including `rusqlite` (bundled, compiles C/C++ from source),
   `tonic`/`prost`, and `sqlite-vec` — even if it only needs domain
   types and the NATS queue.

2. **The CLI and daemon are the same binary.** `vlinder daemon` starts
   the full runtime (workers, storage, container orchestration).
   `vlinder agent run` is a thin client that connects to the daemon
   via gRPC and NATS. These are different programs with different
   dependency profiles shipped as one binary.

## Decision

Split the codebase into a Cargo workspace with three crates:

```
vlindercli/
├── Cargo.toml              (workspace root)
├── crates/
│   ├── vlinder-core/       (library)
│   ├── vlinderd/           (binary)
│   └── vlinder/            (binary)
```

### vlinder-core (library)

The protocol specification. Pure domain types, traits, and queue
implementations. No infrastructure dependencies.

Contains:
- `domain/` — all types, traits, message definitions, workers
- `queue/` — `InMemoryQueue`, `NatsQueue`, `RecordingQueue`

Does NOT contain: config, storage implementations, engine
implementations, gRPC stubs, runtime, CLI.

Dependencies: `serde`, `serde_json`, `chrono`, `sha2`, `uuid`,
`async-nats`, `tokio`, `futures`, `tracing`.

### vlinderd (binary)

The platform daemon. Owns storage, engines, workers, runtime, and
gRPC servers.

Contains:
- `storage/` — SQLite DAG, object, vector, state, registry stores
- `inference/`, `embedding/` — engine implementations (Ollama, OpenRouter)
- `catalog/` — model catalog (Ollama, OpenRouter)
- `runtime/` — container lifecycle (Podman)
- `registry/` — `InMemoryRegistry`, `PersistentRegistry`
- `registry_service/` — gRPC server + conversion
- `state_service/` — gRPC server + conversion
- `services/` — service worker loops
- `supervisor.rs`, `worker.rs`, `worker_role.rs`
- `loader.rs`, `session_server.rs`
- `secret_store/` — implementations (NATS KV, in-memory)
- Own config: worker counts, storage paths, provider endpoints,
  listen addresses

Depends on: `vlinder-core`, `rusqlite`, `sqlite-vec`, `tonic`, `prost`,
`ureq`, `ed25519-dalek`, `rand`, etc.

### vlinder (binary)

The CLI client. Connects to any vlinderd instance.

Contains:
- `harness.rs` — `CliHarness` (build invoke, run agent)
- `commands/` — agent, fleet, timeline, model, secret, repl
- `cli.rs` — Clap argument parsing
- gRPC clients for registry and state services
- Own config: registry address, state address, NATS URL

Depends on: `vlinder-core`, `clap`, `tonic`, `prost`, `reedline`,
`indicatif`, `termimad`, `ureq`.

## Consequences

- Each binary compiles only what it needs. Future binaries (like the
  sidecar) can depend on `vlinder-core` alone.
- The CLI and daemon are independently deployable. `vlinder` can
  connect to any `vlinderd` instance on the network.
- `vlinder-core` can be depended on independently — third parties
  can build tooling against the protocol.
- Config types are separate per binary. Each binary defines its own
  config struct for its own concerns.
- gRPC proto generation needs to be shared. The proto files and
  `tonic-build` step live in a shared location; both `vlinderd`
  (server) and `vlinder` (client) depend on the generated code.
- Tests that span crate boundaries (e.g., recording queue tests
  that use `SqliteDagStore`) become integration tests or use
  in-memory test doubles instead.
