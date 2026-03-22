# ADR 121: Operational Planes

**Status:** Draft

## Context

The platform has three distinct operational concerns that are currently conflated:

1. **Data plane** — agent execution. Invoke, request, response, complete, delegate. Every message is DAG-recorded. Session-scoped. Latency-sensitive. The agent is waiting.

2. **Session plane** — compensating transactions. Fork, repair, promote. Corrective actions applied on top of the immutable execution record. Session-scoped. Deliberate, not real-time.

3. **Infra plane** — provisioning. Deploy, delete. Changes what agents exist and how they're provisioned. Not session-scoped. Currently bypasses the queue entirely (direct gRPC to registry). Status is a read — it queries the registry, not the queue.

Today these are tangled:

- All message types share one `RoutingKey` type with one address format (`vlinder.{session}.{branch}.{submission}.{type}...`), even though infra operations have no session.
- The `Registry` trait mixes infra operations (`register_agent`, `delete_agent`) with data-plane queries (`get_agent`, `get_model`).
- Deploy goes direct to gRPC, bypassing the queue. There's no audit trail, no status tracking, no async lifecycle.
- NATS consumers subscribe to `vlinder.>` and receive all planes — no way to subscribe to data-only or infra-only.

### Why separate now

Issue #15 (async deploy + agent status) requires infra write operations to have their own lifecycle: `submitted → deploying → ready → failed`. This doesn't fit the data-plane model — there's no session, no submission, no DAG recording. Forcing deploy into the existing `RoutingKey` would require fake session/submission IDs. Status is a read from the registry — it doesn't go through the queue.

## Decision

### 1. The plane is the top-level discriminant

Each plane gets its own message address type. The plane determines the address shape — data and session are session-scoped, infra is not. The type hierarchy mirrors the NATS subject hierarchy:

```rust
pub enum RoutingKey {
    Data(DataRoutingKey),
    Session(SessionRoutingKey),
    Infra(InfraRoutingKey),
}
```

### 2. Each plane owns its routing key

**Data plane** — session-scoped, DAG-recorded:
```rust
pub struct DataRoutingKey {
    pub session: SessionId,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub kind: DataMessageKind,
}

pub enum DataMessageKind {
    Invoke { harness, runtime, agent },
    Complete { agent, harness },
    Request { agent, service, operation, sequence },
    Response { service, agent, operation, sequence },
    Delegate { caller, target },
    DelegateReply { caller, target, nonce },
}
```

**Session plane** — session-scoped, DAG-recorded:
```rust
pub struct SessionRoutingKey {
    pub session: SessionId,
    pub branch: BranchId,
    pub submission: SubmissionId,
    pub kind: SessionMessageKind,
}

pub enum SessionMessageKind {
    Repair { harness, agent },
    Fork { agent_name },
    Promote { agent_name },
}
```

**Infra plane** — agent-scoped, audit-logged:
```rust
pub struct InfraRoutingKey {
    pub kind: InfraMessageKind,
}

pub enum InfraMessageKind {
    Deploy { agent_name },
    Delete { agent_name },
}
```

Data and Session share the same address shape today (session, branch, submission) but are separate types. If Session ever needs different fields (e.g. `reason` for audit), it can diverge without touching Data.

### 3. NATS subject prefixes by plane

The subject hierarchy matches the type hierarchy:

| Plane | Subject prefix | Example |
|---|---|---|
| Data | `vlinder.data.v1.{session}.{branch}.{sub}...` | `vlinder.data.v1.abc123.1.sub456.invoke.cli.container.echo` |
| Session | `vlinder.session.v1.{session}.{branch}.{sub}...` | `vlinder.session.v1.abc123.1.sub456.fork.echo` |
| Infra | `vlinder.infra.v1...` | `vlinder.infra.v1.deploy.todoapp` |

Consumers subscribe to `vlinder.data.>` for data only, `vlinder.session.>` for session only, `vlinder.infra.>` for infra only, or `vlinder.>` for everything.

### 4. Separate JetStream streams per plane

Each plane can have its own retention policy:
- **Data**: limits-based retention (bounded by session count)
- **Session**: limits-based (fewer messages, longer retention for decision history)
- **Infra**: interest-based or work-queue (exactly-once delivery for deploy)

### 5. Different planes serve different audiences

Data plane messages are agents doing real work for external users. Session plane messages are developers/operators applying compensating transactions — corrective actions (fork, repair, promote) on top of the immutable execution record. This distinction has implications:

- **Access control**: data plane is open to anyone who can invoke an agent. Session plane (fork, promote) should be restricted to operators — promoting a branch rewrites what "main" means.
- **Audit**: data plane audit is the DAG. Session plane audit is "who forked what, when, why" — a different kind of record about human decisions, not agent behavior.
- **Replay**: data plane messages are replayable (same input, same output). Session plane messages are control actions that change structure, not content. You don't replay a fork.
- **Rate**: data plane is bounded by agent activity. Session plane is bounded by human activity (much lower).
- **Retention**: data plane retention is bounded by session lifecycle. Session plane decisions (fork, promote) may need to be retained indefinitely as decision history.

### 6. Registry trait stays unified for now

The `Registry` trait continues to mix infra and query operations. Splitting the trait is a separate concern from splitting the message planes. The registry is a query interface regardless of which plane initiated the query.

## Open Questions

### Repair: session plane or data plane?

Repair carries `harness` and `agent` — it routes to the agent's sidecar and triggers a service call replay. Fork and Promote are processed by the `RecordingQueue`. Repair behaves more like a specialised Invoke than a session operation. Should it move to the data plane?

## Implementation Strategy

Strangler fig — one message type at a time, each e2e-green:

1. **Add payload type.** Define `FooMessageV2` with only the fields that aren't in the routing key (id, state, diagnostics, payload). No routing fields — those live in `DataRoutingKey`.
2. **Add v2 trait methods.** `send_foo_v2(key, msg)` / `receive_foo_v2(agent)` alongside old methods. Implement in InMemoryQueue (second hashmap), NatsQueue, RecordingQueue.
3. **Add `message_v2` to DagNode.** `Option<ObservableMessageV2>`, always `None` initially. Invariant: exactly one of `message` / `message_v2` is `Some`.
4. **Wire receive side.** Sidecar, lambda, provider handler accept v2 types. Never fires yet — nobody sends v2.
5. **Wire read paths.** `insert_node`, `dag_node_to_proto`, proto converter, CLI commands, git-dag consumer handle `message_v2`. The v2 branch never executes yet — `message_v2` is always `None`.
6. **Wire NATS subject.** Single-source format: subject builder, parser, and filter derive from shared constants. Subject includes protocol version: `vlinder.data.v1.{session}.{branch}.{sub}.{kind}...`
7. **Wire send side (the switch).** Harness constructs `DataRoutingKey` + `FooMessageV2`, calls `send_foo_v2`. Everything is already waiting. **This is the only commit that changes runtime behavior.**
8. **Remove old.** Delete `send_foo`, `receive_foo`, `ObservableMessage::Foo` variant, `From<FooMessage>`, dead match arms, old header serialization.

Each step compiles and passes e2e independently. Steps 1-6 are pure additions — the old path is untouched. Step 7 is the cutover. Step 8 is cleanup.

### Wire format

The v2 wire format separates concerns cleanly:

- **Subject** — routing + protocol version. Parsed without touching the payload.
- **Headers** — NATS concerns only (`Nats-Msg-Id` for dedup). No domain data.
- **Payload** — `serde_json::to_vec(&FooMessageV2)`. Self-contained, no header extraction needed. Diagnostics, state, and all domain metadata live here.

This eliminates `from_nats_headers`, manual header insertion/extraction, and the risk of header size limits for structured data like diagnostics.

## Consequences

- The plane is the top-level type discriminant — code that handles one plane doesn't see the others
- Infra operations (deploy, delete) can go on the queue with their own routing and lifecycle
- Each plane is independently subscribable at the NATS level
- Retention and delivery guarantees can differ per plane
- Data and Session share an address shape but are separate types — can diverge independently
- Infra has a simpler address — no session, branch, or submission
- Agent status tracking (issue #15) fits naturally on the infra plane
- Breaking change to NATS subject format — all consumers need updating
