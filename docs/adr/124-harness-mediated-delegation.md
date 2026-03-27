# ADR 124: Harness-Mediated Delegation

**Status:** Draft

## Context

Delegation today is peer-to-peer. Agent1's sidecar sends a delegate message directly to agent2's sidecar. Agent2 replies directly back. The harness is not in the loop.

```
invoke →[agent1]→ delegate →[agent2]→ delegate-reply →[agent1]→ complete
```

This works, but the platform has no intervention point between the delegate and its execution. You can't:

- Fork from a delegation boundary and substitute agent2's result
- Interpose human approval before agent2 executes
- Replay a delegation with different inputs

The harness drives execution for invoke/complete (ADR 123). Delegation should be no different — it's the same pattern: call a target, get a result.

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

## Consequences

- Every delegation is a forkable, replayable boundary — same as invoke/complete
- HITL plugs in without new message types — any message can carry the flag
- The DAG records human decisions alongside agent behavior
- Agent1 doesn't know or care whether the harness consulted a human before invoking agent2
- Delegation is no longer a special mechanism — it's invoke/complete with a return address
- The sidecar no longer routes delegations; it yields them to the harness
