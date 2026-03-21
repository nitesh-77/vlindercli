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
| Data | `vlinder.data.{session}.{branch}.{sub}...` | `vlinder.data.abc123.1.sub456.invoke.cli.container.echo` |
| Session | `vlinder.session.{session}.{branch}.{sub}...` | `vlinder.session.abc123.1.sub456.fork.echo` |
| Infra | `vlinder.infra...` | `vlinder.infra.deploy.todoapp` |

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

Incremental migration — no big-bang refactoring:

1. **Define new types alongside existing.** Create `DataRoutingKey`, `SessionRoutingKey`, `DataMessageKind`, `SessionMessageKind`. Don't touch `RoutingKey`.
2. **Add `From` conversions.** `From<DataRoutingKey> for RoutingKey` and `From<SessionRoutingKey> for RoutingKey`. New code constructs plane-specific types; the rest of the codebase doesn't change.
3. **Add accessors.** `RoutingKey::as_data()` and `RoutingKey::as_session()` return `Option<&DataRoutingKey>` / `Option<&SessionRoutingKey>`. Call sites that only handle one plane start using these.
4. **Migrate call sites one at a time.** Each function that constructs or matches a `RoutingKey` switches to the plane-specific type. `From` impls keep everything compiling.
5. **Change `RoutingKey` from struct to enum.** When all call sites are migrated, the `From` impls become enum variants.

Each step compiles independently. Each step is committable.

## Consequences

- The plane is the top-level type discriminant — code that handles one plane doesn't see the others
- Infra operations (deploy, delete) can go on the queue with their own routing and lifecycle
- Each plane is independently subscribable at the NATS level
- Retention and delivery guarantees can differ per plane
- Data and Session share an address shape but are separate types — can diverge independently
- Infra has a simpler address — no session, branch, or submission
- Agent status tracking (issue #15) fits naturally on the infra plane
- Breaking change to NATS subject format — all consumers need updating
