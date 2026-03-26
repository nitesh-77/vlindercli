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

Strangler fig — one message type at a time, each e2e-green.

### Per-message migration steps

1. **Typed table + v2 payload type.** Create per-message SQL table (e.g. `request_nodes`). Define `FooMessage` with only payload fields (id, dag_id, state, diagnostics, payload). Routing fields live in `DataRoutingKey`. Wire `get_foo_node` / `insert_foo_node` through DagStore → SQLite → gRPC.
2. **Recording queue.** Switch `send_foo` to write to the typed table via `record_foo` instead of the generic `ObservableMessage` blob path.
3. **DataMessageKind + wire format.** Add variant to `DataMessageKind`. Wire NATS subject (builder, parser, filter). Add `send_foo` / `receive_foo` to MessageQueue trait. Implement in InMemoryQueue, NatsQueue, RecordingQueue.
4. **Git DAG worker.** Add `on_foo` to `DagWorker` trait + `GitDagWorker` impl. Wire into vlinderd's DAG consumer dispatch.
5. **Add v2 receivers.** Service workers / sidecar try v2 first, fall back to v1. Decouples handler logic from v1 types.
6. **Switch senders.** Provider server, sidecar dispatch construct `DataRoutingKey` + v2 payload. **This is the only commit that changes runtime behavior.**
7. **Remove v1.** Delete old trait methods, impls, `ObservableMessage` variants, `From` impls, old header serialization, dead tests. Tree-shake: remove `pub`, let the compiler find dead code, delete, repeat.
8. **Rename.** Drop V2 suffix from types and methods. Clean up stale v2 references in variables, error strings, test names.

Each step compiles and passes e2e independently. Steps 1-5 are pure additions. Step 6 is the cutover. Steps 7-8 are cleanup.

### Progress

| Message type | Status |
|---|---|
| Invoke | ✅ Complete — `InvokeMessage`, `send_invoke`/`receive_invoke` |
| Complete | ✅ Complete — `CompleteMessage`, `send_complete`/`receive_complete` |
| Request | ✅ Complete — `RequestMessage`, `send_request`/`receive_request` |
| Response | ✅ Complete — `ResponseMessage`, `send_response`/`receive_response` |
| Delegate | 🔲 Next — needs `DataMessageKind::Delegate { caller, target, nonce }` |
| DelegateReply | 🔲 Next — needs `delegate_reply_nodes` table, `DataMessageKind::DelegateReply { caller, target, nonce }` |

Note: Delegate currently omits the nonce from its NATS subject (`vlinder.{s}.{b}.{sub}.delegate.{caller}.{target}`). The v2 subject must include it (`vlinder.data.v1.{s}.{b}.{sub}.delegate.{caller}.{target}.{nonce}`) to prevent routing collisions when the same caller delegates to the same target multiple times within a submission.

### Wire format

The v2 wire format separates concerns cleanly:

- **Subject** — routing + protocol version. Parsed without touching the payload.
- **Headers** — NATS concerns only (`Nats-Msg-Id` for dedup). No domain data.
- **Payload** — `serde_json::to_vec(&FooMessage)`. Self-contained, no header extraction needed. Diagnostics, state, and all domain metadata live here.

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
