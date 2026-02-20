# ADR 101: Harness as gRPC Service

**Status:** Draft

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
service HarnessService {
  rpc Deploy (DeployRequest)           returns (DeployResponse);
  rpc Invoke (InvokeRequest)           returns (InvokeResponse);
  rpc Poll   (PollRequest)             returns (PollResponse);
  rpc StartSession (StartSessionRequest)   returns (StartSessionResponse);
  rpc RecordResponse (RecordResponseRequest) returns (RecordResponseResponse);
}
```

### RPC mapping

| Harness method | RPC | Request | Response |
|---|---|---|---|
| `deploy(manifest_toml)` | `Deploy` | manifest TOML string | agent ResourceId |
| `invoke(agent_id, input)` | `Invoke` | agent_id + input + session_id | job_id |
| `poll(job_id)` | `Poll` | job_id | optional result string |
| `start_session(agent)` | `StartSession` | agent name | session_id |
| `record_response(text)` | `RecordResponse` | session_id + response text | ack |

### What stays CLI-local

- `deploy_from_path()` — reads the manifest file from the local filesystem,
  then calls `Deploy` with the TOML string. File I/O is a CLI concern.
- REPL, terminal styling, progress spinners — pure UI.

### What moves to the daemon

- `build_invoke()` — session state, submission ID computation, Merkle
  chaining, queue dispatch. The daemon owns the queue and the session.
- `tick()` — job reconciliation loop. Internal daemon concern.
- `register_agent()` — image digest resolution, registry interaction.
  Image digest resolution uses the Podman REST API exclusively (ADR 077).
  The current `resolve_image_digest()` free function that shells out to
  `podman image inspect` is removed. The daemon constructs a
  `PodmanApiClient` from socket config and uses it for all Podman
  interactions — no CLI shelling.
- Timeline management (`set_timeline`, sealed checks).

### Session ownership

Sessions move to the daemon. The CLI sends `StartSession` once, gets
back a `session_id`, and includes it on every subsequent `Invoke`. The
daemon manages session history, submission chaining, and state tracking.

The CLI keeps a local `Session` only for display purposes (showing
conversation history in the REPL). It does not use it for payload
construction — that's the daemon's job.

## Consequences

- CLI binary depends only on `vlinder-core` (domain types) + gRPC stubs.
  No NATS, SQLite, Podman, or inference engine dependencies.
- Enables the ADR 095 binary split: `vlinder` and `vlinderd` as separate
  crates with minimal shared surface.
- The Harness trait remains the authoritative contract. The gRPC service
  is a serialization of that contract, not a replacement.
- Existing `CliHarness` tests rewritten to use the gRPC client, verifying
  the same behavior through the wire protocol.
