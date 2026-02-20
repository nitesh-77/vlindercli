# ADR 101: Harness as gRPC Service

**Status:** Accepted

## Context

The Harness trait (ADR 029, 076) is the API surface for agent interaction.
Today the only implementation, `CliHarness`, directly depends on
`MessageQueue`, `Registry`, and `runtime::resolve_image_digest()`. This
means the CLI binary must link every infrastructure crate — NATS, SQLite,
Podman, inference engines — even though it only needs to submit jobs and
poll results.

The goal is a clean binary split: `vlinder` (CLI) talks to `vlinderd`
(daemon) exclusively via gRPC. The CLI should know nothing about queues,
storage, or runtimes.

## Decision

Expose the Harness trait as a gRPC service. The daemon runs a
`HarnessService` server that owns the real implementation. The CLI
implements the Harness trait as a thin gRPC client.

### Service definition

```protobuf
service Harness {
  rpc Ping(PingRequest) returns (SemVer);
  rpc SetTimeline(SetTimelineRequest) returns (SetTimelineResponse);
  rpc StartSession(StartSessionRequest) returns (StartSessionResponse);
  rpc SetInitialState(SetInitialStateRequest) returns (SetInitialStateResponse);
  rpc RunAgent(RunAgentRequest) returns (RunAgentResponse);
}
```

### RPC mapping

| Harness method | RPC | Notes |
|---|---|---|
| `start_session(name)` | `StartSession` | Daemon creates session, owns history |
| `set_initial_state(hash)` | `SetInitialState` | Time travel resume from checkpoint |
| `set_timeline(id, sealed)` | `SetTimeline` | Branch-scoped subjects (ADR 093) |
| `run_agent(agent_id, input)` | `RunAgent` | Synchronous invoke→complete (ADR 092) |

Deploy is a registry operation (ADR 103), not a harness operation.
`RunAgent` replaces the old `Invoke`/`Poll` pair — the daemon blocks
internally via `send_and_wait` and the gRPC server uses `spawn_blocking`
to keep h2 alive.

### What stays CLI-local

- `vlinder agent deploy` — reads manifest from disk, registers via
  registry gRPC. File I/O is a CLI concern.
- REPL, terminal styling, progress spinners — pure UI.
- State read via `GrpcStateClient` for `set_initial_state` before run.

### What moves to the daemon

- `CoreHarness` — session state, submission ID, Merkle chaining, queue
  dispatch. The daemon's harness worker owns the queue and registry.
- Timeline management (`set_timeline`, sealed checks).
- All NATS queue interaction — the CLI has no queue dependency.

### Session ownership

Sessions live in the daemon's `CoreHarness`. The CLI calls
`StartSession` once per `agent run`, then `RunAgent` for each REPL
turn. The daemon manages session history, submission chaining, and
state tracking.

## Consequences

- CLI binary depends only on `vlinder-core` (domain types) + gRPC stubs.
  No NATS, SQLite, Podman, or inference engine dependencies.
- Enables the ADR 095 binary split: `vlinder` and `vlinderd` as separate
  crates with minimal shared surface.
- The Harness trait remains the authoritative contract. The gRPC service
  is a serialization of that contract, not a replacement.
- Existing `CliHarness` tests rewritten to use the gRPC client, verifying
  the same behavior through the wire protocol.
