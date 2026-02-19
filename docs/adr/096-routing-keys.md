# ADR 096: Routing Keys as Domain Types

## Status

Draft

## Context

Every entity-to-entity communication in the Vlinder protocol is a **hop**.
There are five hop types corresponding to the five message types:

| Hop | Direction | Message |
|---|---|---|
| Invoke | Harness → Agent | InvokeMessage |
| Request | Agent → Service | RequestMessage |
| Response | Service → Agent | ResponseMessage |
| Complete | Agent → Harness | CompleteMessage |
| Delegate | Agent → Agent | DelegateMessage |

Each hop needs a **routing key** — the queue uses it to deliver the message
to the correct receiver. Today, routing keys are constructed as formatted
strings inside each `MessageQueue` implementation:

```rust
// InMemoryQueue::send_invoke
let subject = format!(
    "vlinder.{}.{}.invoke.{}.{}.{}",
    msg.timeline, msg.submission, msg.harness, msg.runtime.as_str(),
    agent_short_name(&msg.agent_id),
);
```

NatsQueue does the same thing independently. The routing key structure is
implicit — embedded in format strings, duplicated across implementations,
and untestable at the domain level.

### What's wrong

1. **No domain concept for routing.** The protocol says "an invoke is routed
   by timeline + submission + harness + runtime + agent" but this isn't
   expressed as a type. It's scattered across format strings.

2. **Routing correctness is untestable.** The key property — that changing
   any single dimension produces a different routing key — can't be tested
   without reaching into implementation internals (`typed_queues`).

3. **Subject format tests prove nothing.** InMemoryQueue tests verify that
   subject strings contain `.invoke.` and `.cli.`. These are testing
   the serialization format, not routing correctness. And they don't prove
   NatsQueue builds the same subjects.

4. **Harness tests reach into queue internals.** `CliHarness` tests access
   `InMemoryQueue.typed_queues` to inspect subjects and payloads. The
   harness doesn't own subject building — it's over-testing.


## Decision

### 1. RoutingKey — one variant per hop type

A `RoutingKey` enum in the domain captures the dimensions for each hop:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RoutingKey {
    Invoke {
        timeline: TimelineId,
        submission: SubmissionId,
        harness: HarnessType,
        runtime: RuntimeType,
        agent: String,
    },
    Request {
        timeline: TimelineId,
        submission: SubmissionId,
        agent: String,
        service: ServiceType,
        backend: String,
        operation: Operation,
        sequence: Sequence,
    },
    Response {
        timeline: TimelineId,
        submission: SubmissionId,
        service: ServiceType,
        backend: String,
        agent: String,
        operation: Operation,
        sequence: Sequence,
    },
    Complete {
        timeline: TimelineId,
        submission: SubmissionId,
        agent: String,
        harness: HarnessType,
    },
    Delegate {
        timeline: TimelineId,
        submission: SubmissionId,
        caller: String,
        target: String,
    },
}
```

`PartialEq`, `Eq`, and `Hash` derive naturally from all dimensions.

### 2. Messages produce their own routing key

Each message type gets a `routing_key(&self) -> RoutingKey` method. The
message already carries all the dimensions — the routing key just
structures them.

### 3. Trait contract: routing correctness

Domain-level tests enforce the collision-freedom property without knowing
the serialization format:

> For any routing key with N dimensions, if N−1 dimensions are identical
> and 1 differs, the two routing keys MUST NOT be equal.

This is testable purely on `RoutingKey::eq()` — no queue implementation
needed.

### 4. Queue implementations serialize routing keys

`MessageQueue` send methods accept messages (which produce routing keys).
Each implementation serializes the routing key to its native format:

- **InMemoryQueue**: uses `RoutingKey` directly as `HashMap` key (via `Hash`)
- **NatsQueue**: serializes to dot-separated NATS subjects

The serialization format is the implementation's concern. The routing
dimensions are the protocol's concern.

### 5. Receive methods match on routing key dimensions

`receive_request(service, backend, operation)` becomes matching on the
relevant dimensions of the routing key, not substring matching on a
formatted string.

### 6. Subject-format tests move to implementation

InMemoryQueue no longer needs subject-format tests — it keys by
`RoutingKey` directly. NatsQueue tests verify its serialization format.
Harness tests use trait methods, never `typed_queues`.

## Consequences

- Routing dimensions are documented as types, not implicit in format strings
- Collision-freedom is testable at the domain level
- Queue implementations can't accidentally omit a dimension
- `InMemoryQueue.typed_queues` becomes `HashMap<RoutingKey, ...>` — no
  `pub(crate)` field access needed from outside
- Harness tests use `receive_invoke` instead of peeking at internals
- NatsQueue subject format is tested in NatsQueue, not in domain
- Adding a new hop type (future) requires adding a RoutingKey variant —
  the compiler enforces exhaustive handling
