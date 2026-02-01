# ADR 033: Stateless Agent Execution

## Status

Proposed

## Context

ADR 005 established agents as coroutines where "the wasm instance stays in memory while suspended." This works but creates coupling: the runtime must hold WASM instances alive while waiting for service responses.

Current implementation problem: when an agent calls `receive()` to wait for a service response, execution blocks. The harness cannot tick the Provider while the runtime is blocked. Solutions considered:

1. **Tick provider inside receive()** - couples Provider to WasmRuntime
2. **Threading** - run Provider in separate thread
3. **Stateless execution** - agent returns state to host, can be fully destroyed

Option 3 aligns with ADR 032's vision of vlinderd as a distributed registry with separate worker processes.

## Domain Decision

Agents are stateless reducers. On yield, they return their state to the host.

```
process(input, state) → (output, new_state) | (yield, new_state)
```

The WASM instance can be completely destroyed after each call. The host holds the state. On resume, it passes state back in.

```
Agent invoked with (input, None)
  → sends request to service
  → returns (Yield, { waiting_for: "inference-reply", context: ... })
  → WASM instance destroyed

... Provider processes service request ...

Agent invoked with (service_response, Some(saved_state))
  → uses saved_state.context to continue
  → returns (Done, output)
  → WASM instance destroyed
```

## Consequences

**Enables:**
- Zero memory while waiting (WASM fully destroyed)
- Crash recovery (state persisted externally)
- Distribution (state can move between nodes)
- Horizontal scaling (any worker can resume any agent)
- True decoupling of Provider and Runtime

**Requires:**
- Agent authors must make state serializable
- Protocol change: process takes optional state, returns state
- Slight serialization overhead

**Tradeoffs vs ADR 005 (keep WASM alive):**

| Aspect | Keep Alive (ADR 005) | Stateless (this ADR) |
|--------|---------------------|---------------------|
| Memory while waiting | WASM held | Zero |
| State management | Automatic (in WASM) | Explicit (serialize) |
| Crash recovery | State lost | State survives |
| Distribution | Impossible | Enabled |
| Agent complexity | Lower | Higher |

## Current Workaround

Until stateless execution is implemented, we use threading as a workaround:

- WASM execution spawned in a separate thread
- Harness ticks Provider on main thread while WASM blocks
- WASM instance stays alive (per ADR 005)
- Decoupling achieved via threads, not architecture

This unblocks the Provider/Runtime decoupling (ADR 032) without the full stateless rewrite.

## Open Questions

- State serialization format (JSON, MessagePack, protobuf?)
- Maximum state size limits?
- Should simple agents (no yields) be exempt from state management?
