# ADR 005: Agents Are Coroutines

## Status

Accepted (wire protocol updated by ADR 018)

The coroutine model remains valid. The wire protocol changed from `yield_to(agent_ptr, ...)` to queue-based `send(queue, payload, reply_to)` and `receive(queue)`. Implementation uses Extism, not Wasmtime async.

## Context

Agents are wasm. The orchestrator is an agent. Agents need to yield operations and resume with results.

Closures can't cross the wasm boundary. Re-invoking agents loses state. We need a model that:
- Works with wasm
- Preserves agent state across yields
- Keeps orchestrator as just another agent

## Domain Decision

Agents are coroutines. They run, yield operations, suspend, and resume with results.

```
Agent runs
  → yields [op1, op2]
  → suspends (state preserved)
Runtime executes operations
Runtime resumes agent with results
Agent continues
  → yields [op3]
  → suspends
...
Agent returns (done)
```

The wasm instance stays in memory while suspended. State is preserved in linear memory and stack. The runtime resumes it when operations complete.

- Regular agent: runs, returns (single yield)
- Orchestrator: runs, yields, resumes, yields, ... returns (multiple yields)

Same mechanism. Orchestrator just yields more times.

## Wire Protocol

Text boundaries between agents. The contract:

```
yield_to(agent_ptr, agent_len, input_ptr, input_len, response_buf_ptr, response_buf_len) -> response_len
```

- Agent provides: target agent name (string), input (string), response buffer
- Runtime executes target agent, writes response to buffer
- Runtime returns: response length

Strings in, strings out.

## Implementation

Wasmtime async host functions enable suspend/resume:
- `func_wrap_async` for host functions that suspend the caller
- `call_async` to run wasm that may suspend

Agent calls `yield_to`, wasm suspends, runtime executes the operation, wasm resumes with response.

## Consequences

- The execution plan IS the suspended agent - not a separate data structure
- Runtime manages suspend/resume of wasm instances
- Agents import `yield_to` from host to delegate work
