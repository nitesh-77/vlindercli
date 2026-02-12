# ADR 077: Timeline Repair

## Status

Draft

## Context

ADR 075 (State Machine Agents) made every intermediate step capturable. The dispatch loop records the agent's `ctx.state` in every Request commit as `agent-state.json`. The conversations repo now has a complete trace:

```
c1  invoke:   cli → todoapp              (user input)
c2  request:  todoapp → kv.sqlite        agent-state: {"step":"lookup"}
c3  response: kv.sqlite → todoapp        (data)
c4  request:  todoapp → infer.ollama     agent-state: {"step":"infer","data":"..."}
c5  response: infer.ollama → todoapp     [error: connection refused]
c6  complete: todoapp → cli              [error]
```

We can see c4's `agent-state.json` (the agent's progress state when it asked for inference) and c4's `payload` (the exact request parameters: model, prompt, max_tokens). But we can't act on this — we can only observe.

The forcing function: time travel without repair is just a log viewer.

## Decision

**`vlinder timeline repair` re-executes from any Request commit.**

The platform reads the git commit, reconstructs the `AgentAction` that the agent originally returned, re-executes the service call via the bridge, and continues the dispatch loop from there. The agent receives the service response with its saved state — it doesn't know it's a repair.

### Mechanism

Every Request commit contains:
- `agent-state.json` — the agent's `ctx.state` (a JSON `Value`)
- `payload` — the service request parameters (JSON with `op` field)
- Commit message — `request: {agent} → {service}` with `Session:` trailer

From these, the platform reconstructs an `AgentAction`:

| payload `op` | Reconstructed `AgentAction` |
|---|---|
| `kv-get` | `KvGet { path, state }` |
| `kv-put` | `KvPut { path, content, state }` |
| `kv-list` | `KvList { prefix, state }` |
| `kv-delete` | `KvDelete { path, state }` |
| `infer` | `Infer { model, prompt, max_tokens, state }` |
| `embed` | `Embed { model, text, state }` |
| `vector-store` | `VectorStore { key, vector, metadata, state }` |
| `vector-search` | `VectorSearch { vector, limit, state }` |
| `vector-delete` | `VectorDelete { key, state }` |

The reconstructed action is fed into the dispatch loop's existing match logic — the same code path that normally handles actions from POST /handle. The bridge executes the service call, builds the typed `AgentEvent`, and POSTs it to the agent. The agent returns the next action. The loop continues.

### Dispatch refactoring

The existing `dispatch_state_machine()` is split into composable pieces:

```
dispatch_state_machine(port, payload, session, bridge)
    → AgentEvent::Invoke { input, state: {} }
    → dispatch_loop(port, event, session, bridge)

dispatch_resume(port, action, session, bridge)
    → execute_action(action, bridge)
    → dispatch_loop(port, event, session, bridge)

dispatch_loop(port, event, session, bridge)
    → loop { post_handle → execute_action → repeat until Complete }

execute_action(action, bridge)
    → call bridge method → build AgentEvent (or return Done)
```

`execute_action()` is extracted from the existing match block. `dispatch_loop()` is the core loop. `dispatch_resume()` starts from a reconstructed action. The existing `dispatch_state_machine()` wraps these for the normal invoke path.

### Git timeline

Repair creates a branch before running:

```bash
git checkout <commit>              # user navigates to the step
vlinder timeline repair --path .   # platform creates branch and re-executes
```

The repair command creates `repair/<short-hash>` branch from the current HEAD. New commits go on this branch — a fork from the original timeline. The original timeline is untouched.

### What repair is NOT

- Not replay — we don't re-execute all steps from the beginning
- Not rollback — we don't undo anything
- Not resume — we don't need the original container or its call stack
- It's re-execution from a checkpoint — fresh service call, same agent state

## Consequences

- Any Request commit is a resumable checkpoint — not just the last one
- Failed invocations can be repaired after fixing the root cause (restart ollama, fix data, etc.)
- The dispatch loop is more composable — `execute_action()` can be tested independently
- Requires the same agent image to still be available (same code that produced the checkpoint)
- Agent must be deterministic given the same state + event for repair to produce correct results

## Scope

### This ADR

- Dispatch refactoring (execute_action, dispatch_loop, dispatch_resume)
- Git commit reader (reconstruct AgentAction from commit tree)
- `vlinder timeline repair` command

### Follow-on ADRs

- **Batch repair** — repair multiple steps, or repair and run to completion in one command
- **Repair from Response** — re-deliver a successful response without re-executing the service
- **Automated repair** — daemon watches for failures and auto-repairs when services recover
