# ADR 096: Routing Keys as Domain Types

## Status

Accepted

## Context

Every entity-to-entity communication in the Vlinder protocol is a **hop**.
A hop is a request-reply pair: one message out, one message back. There are
three hop types:

| Hop | Request | Reply | Direction |
|---|---|---|---|
| Invoke | InvokeMessage | CompleteMessage | Harness → Agent → Harness |
| Service call | RequestMessage | ResponseMessage | Agent → Service → Agent |
| Delegate | DelegateMessage | CompleteMessage | Agent → Agent → Agent |

Each side of a hop needs a **routing key** — the queue uses it to deliver
the message to the correct receiver. The request routing key gets the
message to the handler. The reply routing key gets the response back to
the caller.

Today, routing keys are constructed as formatted strings inside each
`MessageQueue` implementation. NatsQueue and InMemoryQueue independently
build subject strings from message fields using string interpolation. The
routing key structure is implicit — embedded in format strings, duplicated
across implementations, and untestable at the domain level.

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

5. **Replies aren't modeled.** Request-reply is the fundamental pattern, but
   reply routing keys are implicit. `receive_response` takes a
   `&RequestMessage` and internally builds the reply subject — the pairing
   is hidden in implementation code.

6. **Delegation replies use the NATS inbox pattern.** `create_reply_address`
   on `MessageQueue` generates an opaque string that the caller stuffs into
   `DelegateMessage.reply_subject`. The caller then passes this string to
   `receive_complete_on_subject` to poll for the reply. This is NATS's
   `_INBOX.*` pattern recreated manually in the domain trait — the
   implementations even produce different formats (InMemoryQueue:
   `reply.{sub}.{caller}.{target}.{uuid}`, NatsQueue:
   `vlinder.{sub}.delegate-reply.{caller}.{target}.{uuid}`). Other queue
   systems have no guarantee they can support opaque reply addresses.
   The reply address should be a structured routing key like every other
   message direction.


## Invariants

These are properties of the routing protocol that must hold regardless of
queue implementation.

**1. Collision-freedom.** For any two routing keys of the same variant, if
N−1 dimensions are identical and 1 differs, the routing keys are not equal.
A message intended for agent A must never arrive at agent B.

**2. Reply pairing.** Every request routing key deterministically produces
exactly one reply routing key. The same request always yields the same
reply key.

**3. Reply asymmetry.** Reply routing keys do not have replies. Hops are
one level deep — a request produces a reply, never a chain.

**4. Hop completeness.** Every message direction in the protocol maps to
exactly one RoutingKey variant. No message can be sent without a routing
key.

**5. Nonce uniqueness.** Each delegation produces a unique nonce. Two
delegations between the same caller and target produce different
DelegateReply routing keys. Nonces are content-addressed (ADR 097).


## Decisions

### 1. Collision-freedom is structural

RoutingKey is a composite value type with structural equality and hashing
over all dimensions. Two routing keys are equal if and only if they are
the same variant and every dimension matches. In a strongly typed
language, collision-freedom is guaranteed by the compiler — not by
implementation discipline or testing.

Queue implementations serialize routing keys to their native format. The
serialization must be **injective** — two different routing keys must
never map to the same serialized value:

- **InMemoryQueue**: uses `RoutingKey` directly as `HashMap` key.
  Injectivity is free — no serialization occurs.
- **NatsQueue**: serializes to dot-separated NATS subjects. Injectivity
  is testable on the serializer.

Implementations cannot break collision-freedom because they never
construct routing keys — they only serialize them.

### 2. RoutingKey — one variant per message direction

A routing key is a composite value type. Each variant corresponds to one
message direction in the protocol. Equality is structural — two routing
keys are equal if and only if they are the same variant and every
dimension matches.

| Variant | Dimensions |
|---|---|
| Invoke | timeline, submission, harness, runtime, agent |
| Complete | timeline, submission, agent, harness |
| Request | timeline, submission, agent, service-backend, operation, sequence |
| Response | timeline, submission, service-backend, agent, operation, sequence |
| Delegate | timeline, submission, caller, target |
| DelegateReply | timeline, submission, caller, target, nonce |

Agents are identified by `AgentId` — a routing-context identity type,
distinct from the registration context's `ResourceId`.

Service-backend is a single typed dimension that scopes backends to their
service type. Each service type defines its own set of valid backends:

| Service | Backends |
|---|---|
| Kv | Sqlite, InMemory |
| Vec | SqliteVec, InMemory |
| Infer | Ollama, OpenRouter |
| Embed | Ollama |

Invalid combinations (e.g., Kv + Ollama) are unrepresentable.

### 3. Hop — paired routing keys

A hop pairs a request routing key with its reply routing key. Given a
request routing key, the reply routing key is deterministic — it shares
the common dimensions and rearranges them for the reply direction.

| Request variant | Reply variant | Additional dimensions |
|---|---|---|
| Invoke | Complete | — |
| Request | Response | — |
| Delegate | DelegateReply | nonce (unique per delegation) |

Calling reply on a reply variant is undefined. Hops are one level deep.

### 4. Messages produce their own routing key

Each message type produces a routing key. The message already carries all
the dimensions — the routing key just structures them. This is a pure
projection, not a computation.

### 5. Queue implementations serialize routing keys

`MessageQueue` send methods accept messages (which produce routing keys).
Each implementation serializes the routing key to its native format:

- **InMemoryQueue**: uses `RoutingKey` directly as `HashMap` key (via `Hash`)
- **NatsQueue**: serializes to dot-separated NATS subjects

The serialization format is the implementation's concern. The routing
dimensions are the protocol's concern.

### 6. Receive methods use reply keys (enforces invariant 2)

`receive_response(&self, request: &RequestMessage)` becomes: compute
`request.routing_key().reply_key()`, look up messages at that key. The
pairing logic moves from queue internals to the domain type.

### 7. Delegation replies use routing keys, not opaque addresses (enforces invariant 5)

`create_reply_address`, `send_complete_to_subject`, and
`receive_complete_on_subject` are removed from `MessageQueue`. Delegation
replies route via `RoutingKey::DelegateReply` like every other message
direction. The nonce provides uniqueness — same role as the UUID in
`create_reply_address`, but as a dimension of a structured routing key
rather than an embedded substring of an opaque string.

`DelegateMessage.reply_subject: String` becomes a `RoutingKey` (or the
dimensions needed to reconstruct one). The caller doesn't need to know the
serialization format — it just passes the routing key to the queue.

### 8. Subject-format tests move to implementation (consequence of 1)

InMemoryQueue no longer needs subject-format tests — it keys by
`RoutingKey` directly. NatsQueue tests verify its serialization format.
Harness tests use trait methods, never `typed_queues`.

## Consequences

- Hops are explicit: three request-reply pairs, each with paired routing keys
- Routing dimensions are documented as types, not implicit in format strings
- Collision-freedom and reply pairing are testable at the domain level
- Queue implementations can't accidentally omit a dimension
- `InMemoryQueue.typed_queues` becomes `HashMap<RoutingKey, ...>` — no
  `pub(crate)` field access needed from outside
- Harness tests use `receive_invoke` instead of peeking at internals
- NatsQueue subject format is tested in NatsQueue, not in domain
- `create_reply_address`, `send_complete_to_subject`, and
  `receive_complete_on_subject` are eliminated — delegation replies use
  the same routing key mechanism as every other hop
- Adding a new hop type (future) requires adding RoutingKey variants —
  the compiler enforces exhaustive handling
