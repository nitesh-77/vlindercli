# ADR 121: Operational Planes

**Status:** Draft

## Context

The platform has three distinct operational concerns that are currently conflated:

1. **Data plane** — agent execution. Invoke, request, response, complete, delegate. Every message is DAG-recorded. Session-scoped. Latency-sensitive. The agent is waiting.

2. **Session plane** — history manipulation. Fork, repair, promote. User-initiated actions that mutate the DAG structure (branches). Session-scoped. Deliberate, not real-time.

3. **Infra plane** — provisioning. Deploy, delete. Changes what agents exist and how they're provisioned. Not session-scoped. Currently bypasses the queue entirely (direct gRPC to registry). Status is a read — it queries the registry, not the queue.

Today these are tangled:

- All message types share one `RoutingKey` type with one address format (`vlinder.{session}.{branch}.{submission}.{type}...`), even though infra operations have no session.
- The `Registry` trait mixes infra operations (`register_agent`, `delete_agent`) with data-plane queries (`get_agent`, `get_model`).
- Deploy goes direct to gRPC, bypassing the queue. There's no audit trail, no status tracking, no async lifecycle.
- NATS consumers subscribe to `vlinder.>` and receive all planes — no way to subscribe to data-only or infra-only.

### Why separate now

Issue #15 (async deploy + agent status) requires infra write operations to have their own lifecycle: `submitted → deploying → ready → failed`. This doesn't fit the data-plane model — there's no session, no submission, no DAG recording. Forcing deploy into the existing `RoutingKey` would require fake session/submission IDs. Status is a read from the registry — it doesn't go through the queue.

## Decision

### 1. Three routing key types

Each plane gets its own routing key type with its own addressing:

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

### 2. NATS subject prefixes by plane

Each plane gets its own subject prefix for clean subscription filtering:

| Plane | Subject prefix | Example |
|---|---|---|
| Data | `vlinder.data.{session}.{branch}.{sub}...` | `vlinder.data.abc123.1.sub456.invoke.cli.container.echo` |
| Session | `vlinder.session.{session}.{branch}.{sub}...` | `vlinder.session.abc123.1.sub456.fork.echo` |
| Infra | `vlinder.infra...` | `vlinder.infra.deploy.todoapp` |

Consumers subscribe to `vlinder.data.>` for data only, `vlinder.infra.>` for infra only, or `vlinder.>` for everything.

### 3. Separate JetStream streams per plane

Each plane can have its own retention policy:
- **Data**: limits-based retention (bounded by session count)
- **Session**: limits-based (fewer messages, longer retention)
- **Infra**: interest-based or work-queue (exactly-once delivery for deploy)

### 4. Existing RoutingKey splits into Data + Session

The current `RoutingKey` struct becomes `DataRoutingKey`. `RoutingKind` splits into `DataMessageKind` (Invoke, Complete, Request, Response, Delegate, DelegateReply) and `SessionMessageKind` (Repair, Fork, Promote). The common fields (session, branch, submission) stay on both structs — they share the same address shape.

### 5. Registry trait stays unified for now

The `Registry` trait continues to mix infra and query operations. Splitting the trait is a separate concern from splitting the message planes. The registry is a query interface regardless of which plane initiated the query.

## Open Questions

### Repair: session plane or data plane?

Repair carries `harness` and `agent` (an `AgentId` routing identity) — it routes to the agent's sidecar and triggers a service call replay. Fork and Promote carry `agent_name` (a `String` from user input) and are processed by the `RecordingQueue`. Repair behaves more like a specialised Invoke than a session operation. Should it move to the data plane?

### Inconsistent agent identity across session operations

Repair uses `AgentId`, Fork and Promote use `String` for the agent. All three are session operations on an agent — they should use the same identifier type. This predates the plane split and should be resolved when implementing.

## Consequences

- Infra operations (deploy, delete) can go on the queue with their own routing and lifecycle
- Each plane is independently subscribable at the NATS level
- Retention and delivery guarantees can differ per plane
- The `RoutingKey` type splits but the address format for data/session is unchanged (just prefixed)
- InfraRoutingKey has a simpler address — no session, branch, or submission
- Agent status tracking (issue #15) fits naturally on the infra plane
- Breaking change to NATS subject format — all consumers need updating
