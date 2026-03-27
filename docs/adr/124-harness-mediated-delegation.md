# ADR 124: Harness-Mediated Delegation

**Status:** Draft

## Context

Delegation today is peer-to-peer. Agent1's sidecar sends a delegate message directly to agent2's sidecar. Agent2 replies directly back. The harness is not in the loop.

```
invoke →[agent1]→ delegate →[agent2]→ delegate-reply →[agent1]→ complete
```

This has two problems: a design limitation (no intervention points) and a correctness bug (consumer collision).

### Design limitation: no intervention points

The platform has no intervention point between the delegate and its execution. You can't:

- Fork from a delegation boundary and substitute agent2's result
- Interpose human approval before agent2 executes
- Replay a delegation with different inputs

The harness drives execution for invoke/complete (ADR 123). Delegation should be no different — it's the same pattern: call a target, get a result.

### Correctness bug: cross-sidecar consumer collision

Tested with the council fleet (orchestrator + 3 advisors). The orchestrator delegates to 3 advisors in parallel; each advisor calls OpenRouter for inference. Multi-round: 3 rounds × 3 advisors = 9 delegations.

**Symptom:** Orchestrator stuck in `handle_wait` polling for a delegate-reply that never arrives. NATS stream contains delegate-reply messages, but with nonces that don't match what the orchestrator expects.

**Root cause:** `response_filter()` (`vlinder-nats/src/queue.rs:990`) constructs a response consumer filter with a wildcard for the agent name:

```
vlinder.data.v1.*.*.{submission}.response.*.{service_type}.{backend}.{operation}.{sequence}
```

The `*` at the agent position means ALL advisor sidecars sharing the same submission create (or reuse) the **same named JetStream consumer**. Named consumers are global — `filter_to_consumer_name()` (`queue.rs:1304`) produces identical names for all three advisors because the filter is identical. Whichever sidecar calls `receive_response` first steals the response, even if it was intended for a different advisor.

**Call chain that creates the shared consumer:**

1. Orchestrator's provider server sends delegate via `send_delegate` → `handler.rs:180`
2. Advisor's sidecar receives delegate → `sidecar.rs:166`
3. Advisor's sidecar calls `handle_invoke` → `dispatch.rs:66`, which starts a provider server per invoke
4. Agent calls inference → provider server calls `call_service` → `message_queue.rs:191`
5. `call_service` calls `receive_response(submission, service, operation, sequence)` → `message_queue.rs:211`
6. `receive_response` calls `response_filter()` → `queue.rs:429` → `queue.rs:990`
7. `response_filter` uses `*` for agent → produces identical filter for all advisors
8. `get_or_create_consumer` → `queue.rs:111` → creates a named consumer from the filter
9. All three advisor sidecars share this consumer; responses go to whoever polls first

**Observed failure (2026-03-27, council fleet e2e):**

NATS stream sequence trace (37 messages):
- Round 1 (#2-#13): 3 delegates sent, 3 advisor inference calls, all 3 replies arrive. Nonces match because timing happened to work — each sidecar got its own response before another could steal it.
- Round 2 (#16-#27): 3 delegates, 3 inference calls. Responses start arriving out of order. Architect's inference response (#26) is consumed by product-advisor's sidecar, which sends a delegate-reply with product's nonce (#27). Nonce mismatch doesn't cause failure yet because the orchestrator waits sequentially and the right replies eventually arrive.
- Round 3 (#30-#37): 3 delegates, 3 inference requests. Only 1 of 3 inference responses arrives (#36 for architect). The other two responses never appear in the stream — the inference worker likely responded, but the response was consumed by the wrong sidecar's stale consumer.

The orchestrator's `handle_wait` (`handler.rs:188`) polls `receive_delegate_reply` for nonce `f575f4e4`. The sales-advisor's delegate-reply for that nonce was never sent because its inference response was stolen by another sidecar. Result: infinite poll loop, 8000+ iterations, 900+ seconds.

**Why it worked before:** The race condition depends on timing. When advisors respond at different speeds (different inference latencies), each sidecar happens to poll when its own response is next in the consumer. When response times overlap, the wrong sidecar wins the poll.

**What this confirms about the design:**

The peer-to-peer delegation model has no way to isolate response routing per agent within a shared submission. The `receive_response` signature (`message_queue.rs:97`) takes `(submission, service, operation, sequence)` — no agent name. Including the agent name would fix the consumer collision but not the design limitation (no intervention points). Both problems are solved by harness-mediated delegation: the harness owns all routing, each delegation is its own invoke/complete pair with no shared consumers.

## Decision

### 1. The harness mediates all delegations

When agent1 yields a delegate, the harness receives it — not agent2's sidecar. The harness then invokes agent2 as a normal invoke. When agent2 completes, the harness wraps the result as a delegate-reply and sends it back to agent1.

```
invoke →[agent1]→ delegate →[harness]→ invoke →[agent2]→ complete →[harness]→ delegate-reply →[agent1]→ complete
```

Every arrow is a DAG-recorded message. Every boundary is a point where the platform can fork, substitute, or interpose.

### 2. Nested delegation is recursive

Agent2 can itself delegate to agent3. The harness handles it the same way — receive delegate, invoke agent3, receive complete, send delegate-reply to agent2. No special nesting logic. The harness is always in the middle.

### 3. HITL is a per-message flag

Human-in-the-loop is not a message type or a routing path. It's a flag on any message. Before executing any message (invoke, delegate, request), the harness checks the flag. If set:

1. Harness pauses execution
2. Routes to a human for approval (via web UI, Slack, email — the channel is configuration)
3. Records the human's decision in the DAG
4. Proceeds or aborts based on the decision

The message types don't change. The routing doesn't change. HITL is a harness-level concern, orthogonal to the message protocol.

## Implementation Strategy

### What to delete (peer-to-peer delegation plumbing)

The current peer-to-peer routing code is broken (consumer collision) AND being replaced. Delete, don't fix.

**MessageQueue trait** — `message_queue.rs:107-136`:
- `send_delegate`, `receive_delegate`, `send_delegate_reply`, `receive_delegate_reply`

**Queue impls:**
- InMemoryQueue — `in_memory.rs:212-276`
- NatsQueue — `vlinder-nats/src/queue.rs:446-595`
- RecordingQueue — `recording.rs:380-455`

**Provider server (agent-side delegation):**
- `handler.rs:119-236`: `handle_runtime`, `handle_delegate`, `handle_wait`
- `handler.rs:29`: `pending_replies` field on `InvokeHandler`
- `provider_server.rs:115`: runtime host dispatch branch

**Sidecar (target-side delegation):**
- `sidecar.rs:166-202`: `receive_delegate` branch in main loop
- `dispatch.rs:47`: `reply_key` field on `DurableSession`
- `dispatch.rs:76`: `reply_key` parameter on `handle_invoke`
- `dispatch.rs:418`: `reply_key` parameter on `handle_action`
- `dispatch.rs:543-574`: `send_reply` function (delegate-reply branch)

### What to keep (domain types)

- `DelegateMessage` — `vlinder-core/src/domain/message/delegate.rs`
- `DelegateReplyMessage` — `vlinder-core/src/domain/message/complete.rs`
- `DelegateDiagnostics` — `vlinder-core/src/domain/diagnostics.rs`
- `delegate_nodes` SQL table — `vlinder-sql-state/src/dag_store.rs:108-117`
- `ObservableMessage::Delegate` / `ObservableMessage::DelegateReply` variants

### What to build (harness-mediated path)

The harness receives delegates and routes them. No sidecar-to-sidecar communication.

1. **DataMessageKind::Delegate** — add to `routing_key.rs`. Include nonce in the subject to prevent routing collisions (the v1 delegate subject omits the nonce).
2. **DataMessageKind::DelegateReply** — add to `routing_key.rs`. Already has nonce in subject.
3. **MessageQueue send/receive** — data-plane delegate methods using `DataRoutingKey`.
4. **Harness delegation handler** — receives delegate from agent1's provider server, invokes agent2 via `run_agent`, receives complete, sends delegate-reply back to agent1.
5. **Provider server** — `handle_delegate` sends delegate on the data plane (not peer-to-peer). `handle_wait` receives delegate-reply from the harness.
6. **Sidecar** — remove `receive_delegate` from main loop. The sidecar no longer receives delegations; only the harness does.
7. **DagWorker** — `on_delegate` / `on_delegate_reply` typed hooks.
8. **DagStore** — `insert_delegate_node` / `insert_delegate_reply_node`.

## Consequences

- Every delegation is a forkable, replayable boundary — same as invoke/complete
- HITL plugs in without new message types — any message can carry the flag
- The DAG records human decisions alongside agent behavior
- Agent1 doesn't know or care whether the harness consulted a human before invoking agent2
- Delegation is no longer a special mechanism — it's invoke/complete with a return address
- The sidecar no longer routes delegations; it yields them to the harness
- The consumer collision bug is eliminated — the harness owns all response routing, no shared consumers across sidecars
