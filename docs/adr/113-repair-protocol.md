# ADR 113: Repair Protocol

**Status:** Draft

## Context

Durable execution (ADR 111) lets agents make service calls through named
checkpoints. When a service call fails (timeout, error), the conversation
is stuck — the agent never received the response and can't continue.

Repair means: fork from the failure point, re-execute the service call,
and hand the result to the agent's checkpoint handler so it can continue.

Prerequisites already landed:
- `dag_parent` on `InvokeMessage` (overrides Merkle chain parent in RecordingQueue)
- Checkpoint stored on `DagNode` (SQL) and in git (subtree file + commit trailer)
- Sessions as islands in git (per-session orphan commits, `refs/sessions/<id>`)
- Canonical hash stored in git message subtrees

## Design Decisions

### DAG parent semantics

In normal operation:
- **New session**: `dag_parent` is empty → root of a new Merkle chain
- **Ongoing session**: `RecordingQueue` chains automatically from its cache
  (or SQL fallback via `latest_node_hash()`)

The `dag_parent` field on `InvokeMessage` is an explicit override. When
non-empty, `RecordingQueue` uses it instead of the chain cache. Repair
is the first consumer of this override.

In git, the parallel:
- **New session**: orphan commit, creates `refs/sessions/<session_id>`
- **Ongoing session**: `session_commit()` resolves the ref, last commit
  becomes the git parent

### Checkout as the entry point

Checkout is a cursor move — it positions the user at a point in the
DAG without mutating anything. Any subsequent action that creates new
nodes will parent from the checked-out point.

### Checkout point types

A checkout point is either:

1. **Logically complete** — the agent finished a full round trip
   (Invoke→...→Complete). State is clean. The only action from here
   is a new invoke (next conversation turn).

2. **Not logically complete** — the agent was mid-round-trip. A service
   call may have failed, or the agent was partway through its work.

Repair only applies to case 2, and only when the message has a
checkpoint. The checkpoint is the agent's named re-entry point — without
it, the platform has nowhere to deliver a replayed service response.

If the agent didn't use checkpoints (unmanaged mode), the only option
from a non-complete point is to discard and re-invoke from the last
complete.

### Repair is a replay

At a mid-execution checkpoint, there is no user-facing state to inspect.
The intermediate data (inference prompt, kv operation) is internal
plumbing. The user's only decision is "retry this."

The platform already has the full request in the DAG — service, backend,
operation, payload, checkpoint handler. Repair is: re-send the same
request to the same service and deliver the response to the checkpoint
handler. No new user input. No modified payload. Just replay.

### Normal vs repair execution flow

Normal execution — the agent initiates the service call:

```
Agent → Sidecar → Service → Sidecar → Agent
```

Repair — the platform initiates the service call, the agent only
enters the picture when the response arrives at the checkpoint handler:

```
Platform → Sidecar → Service → Sidecar → Agent (checkpoint handler)
```

The sidecar skips the agent on the way out and re-joins the agent on
the way back.

### State restoration

The agent's checkpoint handler runs inside a container that may have
started fresh. In-memory state from the original execution is gone.
The State: trailer on DAG commits captures the agent's state at each
point in the conversation.

Checkout restores this state (via `set_checkout_state`). Since checkout
is a precondition for repair — repair without checkout is prohibited —
state restoration is already handled. Repair itself has no state work
to do.

### RepairMessage

RepairMessage is an instruction to the sidecar: "make this service call
and deliver the result to this checkpoint handler."

It carries the service call metadata from the original failed request
(read from the DAG): service, backend, operation, payload, checkpoint.
The sidecar receives it, constructs a `RequestMessage` using the
provider server's existing queue plumbing, sends it through the queue,
gets the `ResponseMessage` back, and delivers the response payload to
the agent's `/handle?checkpoint=<name>` endpoint.

No new message type is needed for the service call itself — it's a
regular `RequestMessage`. The only difference from normal execution is
that the sidecar initiates the call (from RepairMessage metadata)
instead of the provider server intercepting an agent HTTP call.

### Session identity

Repair stays in the same session — it's one conversation that branched.
The session gains a second branch in git via multiple refs under
`refs/sessions/<id>/`.

After the repair completes, the user promotes the repaired branch to
main using `vlinder session promote`:

1. `vlinder session list <agent-name>` — find the session
2. `vlinder session get <session-id>` — browse turns and messages,
   identify the failed request by its canonical hash
3. `vlinder session fork <session-id> --from <hash> --name <branch>`
   — create a named timeline from the failed point
4. `vlinder session repair <branch>` — replay the failed service call
5. `vlinder session promote <branch>` — make the repaired branch
   canonical, seal the old one

The branch name is the handle that ties fork → repair → promote
together. Multiple forks of the same session can exist independently.

The canonical hash visible in `session get` output is the bridge
between the user's view and the SQL DAG store. Fork uses it to
identify the exact message to branch from.

No new machinery for branch resolution. The existing Timeline entity
(branch_name, parent_timeline_id, fork_point, broken_at) handles it.

### Git ref naming

Session refs gain one level of nesting:

- `refs/sessions/<session_id>/main` — the canonical branch (was `refs/sessions/<session_id>`)
- `refs/sessions/<session_id>/repair-<yyyymmddhhmmss>` — repair branches

Promote moves the `main` ref to the repair branch tip and renames
the old main to indicate it's broken.

### Agent awareness

The agent does not know it's a repair. The checkpoint handler receives
a service response and processes it identically whether it's the
original execution or a replay.

### NATS routing

RepairMessage gets its own NATS subject, separate from invoke. The
sidecar subscribes to both. Separate subjects give clean auditability
— repair traffic is independently filterable and traceable.

### RepairMessage structure

RepairMessage carries everything the sidecar needs to construct and
send a `RequestMessage` through the queue. It is close to a
`RequestMessage` — same service call metadata — plus routing fields
(`harness`, `dag_parent`) and minus `diagnostics` (no agent action
to observe; the agent didn't initiate this call).

```
RepairMessage {
    id, protocol_version, timeline, submission, session,
    agent_id, harness,
    dag_parent,          // canonical hash of fork point (required)
    checkpoint,          // handler name (required)
    service, operation, sequence,
    payload,
    state,
}
```

- `dag_parent` and `checkpoint` are required (never empty) — enforced
  by the type, unlike InvokeMessage where `dag_parent` is optional.
- `sequence` carries through from the original request (it's a retry
  of the same logical call, not a new first call).
- `state` carries through from the original request.
- `diagnostics` excluded — there is no agent-side action to observe.
  The sidecar records fresh `ResponseDiagnostics` when the service
  actually executes.
- Reply type: `CompleteMessage`, same as `InvokeMessage`.

## Implementation

### Domain types (vlinder-core)

- `RepairMessage` struct — 6th message type
- `ObservableMessage::Repair` variant
- `RoutingKey::Repair` variant — NATS subject routing
- `MessageType::Repair` — DagNode enum variant

### Git ref naming (vlinder-git-dag)

- `refs/sessions/<id>/main` — rename from `refs/sessions/<id>`
- Update `session_commit()`, `session_canonical_hash()`,
  `on_observable_message()`

### NATS plumbing (vlinder-nats)

- Send/receive RepairMessage on its own subject
- Headers and serialization

### Sidecar (vlinder-podman-sidecar)

- Subscribe to repair subject alongside invoke
- `handle_repair()` — construct RequestMessage from RepairMessage,
  send through queue, deliver response to `/handle?checkpoint=<name>`

### Harness (vlinder-harness, vlinder-core)

- `repair_agent()` on Harness trait
- gRPC `RepairAgent` RPC — proto, server, client

### CLI (vlinder)

New `vlinder session` and `vlinder turn` subcommands:

- `vlinder session list <agent-name>` — list sessions from DagStore
- `vlinder session get <session-id>` — show turns with messages,
  canonical hashes visible for each message
- `vlinder session fork <session-id> --from <hash> --name <branch>`
  — create Timeline row, set fork_point to canonical hash
- `vlinder session repair <branch>` — read fork point from Timeline,
  look up DagNode by canonical hash, build RepairParams, call
  `harness.repair_agent()`
- `vlinder session promote <branch>` — seal old timeline, rename
  repaired branch to main
- `vlinder turn get <submission-id>` — show all messages in a turn
