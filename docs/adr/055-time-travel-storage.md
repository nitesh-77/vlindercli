# ADR 055: Version-Controlled Agent State

**Status:** Accepted

## Context

ADR 054 made `SubmissionId` = git commit SHA. Every `RequestMessage` carries a SHA representing a position in a causal graph. Storage engines receive this on every operation but ignore it — they overwrite state on each write.

This means storage is destructive and non-isolated. If an agent crashes mid-invocation after 23 of 40 writes, the first 22 writes are permanently applied — half-finished work is visible. If two agents write to overlapping keys, they interleave unpredictably. There is no rollback, no isolation, no atomicity.

Git already solves this problem — for source code. The same model solves it for agent state. Not git the tool. Git the model: immutable objects, content addressing, mutable pointers.

## Decision

### Three Rules

**1. Immutable objects.** Writes create new objects. Old objects are never modified or deleted. A `kv_put` does not overwrite — it appends a new version. The old version still exists.

**2. Content addressing.** Every object is identified by a hash of its content and lineage. Same content in the same context produces the same hash. The platform computes the hash — agents receive it as the return value of each write.

**3. Platform-managed pointers.** "Current state" is a mutable pointer (a ref) managed by the platform. The data doesn't move — the perspective does.

### The Object Model

Git has blobs (content), trees (snapshots), and commits (tree + parent). Agent state uses the same structure:

| Git | Agent state | Purpose |
|-----|-------------|---------|
| Blob | Value | The data — JSON, vector, metadata |
| Tree | Snapshot | Mapping of all paths → value hashes at a point in time |
| Commit | State commit | Snapshot hash + parent commit hash |
| HEAD | Head pointer | The platform's ref to the agent's current state |

### Writes

A single mutation — `kv_put("/todos.json", new_data)`:

```
1. Hash new_data                          → value_hash
2. Clone parent snapshot, update entry    → snapshot' { "/todos.json" → value_hash }
3. Create state commit: snapshot' + parent → commit_b

Before:  HEAD → commit_a → { "/todos.json" → old_hash }
After:   HEAD → commit_b → { "/todos.json" → value_hash }
         commit_a still exists. old_hash still exists.
```

Multiple mutations chain naturally:

```
HEAD → commit_a

kv_put("/todos.json", v2)       → commit_b
embed("chunk-0", vec, meta)     → commit_c
kv_put("/config.json", cfg)     → commit_d

Success: HEAD → commit_d   (pointer advances)
Failure: HEAD → commit_a   (pointer doesn't move)
```

### Reads

`kv_get("/todos.json")` resolves through the current pointer:

```
HEAD → state commit → snapshot → "/todos.json" → value_hash → content
```

Within an invocation, the pointer advances locally — agents see their own writes. Other agents and sessions see the last committed head. This is snapshot isolation, emergent from immutability.

### Agent Invocations

Each invocation is a work unit:

- **Start.** Platform reads the session's head pointer. The agent inherits this state.
- **During.** Each mutation extends the chain from the head. The pointer advances locally.
- **Success.** Platform advances the session's head to the final state commit.
- **Failure.** Platform does not advance the head. Orphaned objects are eligible for GC.

There is no `begin()`. No `commit()`. No `rollback()`. The pointer either moves forward or it doesn't.

### Cross-Turn Continuity

The conversation DAG (git) and the state DAG (object model) are two timelines with the same structure. They link at the turn boundaries:

```
conversation:  ── user1 ──── agent1 ──── user2 ──── agent2 ──
                    │           ↑           │           ↑
                    │        advance        │        advance
                    ▼        head           ▼        head
state:              └─ a→b→...→z           └─ d→e→...→y
```

The head pointer after each agent turn is recorded as a `State` trailer on the agent response commit:

```
agent

This article discusses...

Session: ses-abc123
Submission: aaa
State: zzz
```

### Platform-Provided Storage

For agents using the platform's kv and vector bridges, the platform implements the three rules transparently. Agents call `kv_put` / `kv_get` / `vector_store` / `vector_search` exactly as before. The platform handles hashing, snapshots, state commits, and pointer management behind the scenes.

`kv_put` returns a hash string instead of `"ok"`. Backwards compatible — agents can ignore it. Agents that want fine-grained references can store these hashes.

### Agent-Provided Storage

Agents that bring their own storage follow the same three rules in their chosen backend:

- Append-only writes (never overwrite or delete)
- Content-addressed objects (keyed by hash)
- Report state commit hashes back to the platform

The platform publishes the hash algorithm. The platform manages the head pointer. The agent manages the data. Any backend works — the contract is the model, not the mechanism.

### What Falls Out

These are not features to build. They are consequences of the three rules:

| Capability | Mechanism |
|------------|-----------|
| **Crash safety** | Pointer doesn't advance on failure |
| **Isolation** | Concurrent work units write to different chains, reads scoped to each head |
| **Undo** | Move pointer backward |
| **Time travel** | Point at any historical state commit |
| **Fork** | Two pointers from the same parent |
| **Diff** | Compare snapshots at two state commits |
| **Audit** | Walk the commit chain |
| **Replay** | Start a new chain from an old state commit |
| **Collaboration** | Share the object store and commit chain |

### State Continuity

New sessions automatically pick up where the last session left off. On `vlinder agent run`, the platform scans backwards from HEAD for the most recent `State` trailer for that agent and initializes the harness with it.

`git log` naturally follows the current branch. After a `timeline fork`, agents see state from the fork point — no special logic needed.

### User Experience: Control Plane, Not Data Plane

Time travel is a platform operation, not an agent interaction. The agent never knows about history, forking, or undo — it reads and writes through the pointer. The user navigates the timeline from outside the REPL using `vlinder timeline` subcommands.

**The conversation repo is system-wide.** Every commit — from any agent, any session — lives in the same git DAG. Forking is a system-wide operation — it affects all agents because they share the same timeline. Git branches are the natural mechanism.

```
$ vlinder timeline log
abc1234  todoapp    ses-001  Turn 3: add clean the house     State: ddd
def5678  pensieve   ses-002  Turn 1: summarize article        State: aaa
ghi9012  todoapp    ses-001  Turn 4: complete all             State: eee
jkl3456  pensieve   ses-002  Turn 2: what about point 3?      State: bbb
mno7890  todoapp    ses-001  Turn 5: delete completed         State: fff

$ vlinder timeline log --agent pensieve   # filter to one agent
```

**Inspect state at any point:**

```
$ vlinder timeline diff abc1234 mno7890
  todoapp /todos.json: 3 items → 0 items
  pensieve /articles.json: 0 articles → 1 article
```

**Fork the system timeline:**

```
$ vlinder timeline fork abc1234
Forked timeline at abc1234 → branch fork-abc12345

$ vlinder agent run todoapp
Resuming from state ddd…
> list
1. buy milk
2. walk the dog
3. clean the house
```

The fork creates a new git branch in the conversation repo from the target commit. All subsequent `agent run` sessions land on this branch. The old timeline is preserved on the old branch.

| Plane | Tool | Does what |
|-------|------|-----------|
| Control | `vlinder timeline log` | Walk the system-wide commit chain |
| Control | `vlinder timeline fork` | Fork the system timeline at any commit |
| Control | `vlinder timeline diff` | Compare snapshots at any two commits |
| Data | The REPL | Talk to the agent — no awareness of time travel |

### Garbage Collection

Append-only means storage grows. Unreachable objects — from failed invocations or history beyond a retention window — are eligible for GC. Retention policy is a separate concern (future ADR).

## Scope

### Done

- KV storage versioning (content-addressed append-only SQLite)
- State tracking through the invocation lifecycle (ServiceRouter → Worker → messages → git trailers)
- `vlinder timeline log` — system-wide timeline with state hashes (replaces `vlinder session log`)
- `vlinder timeline fork <commit>` — system-wide fork via git branch (replaces `--from` on `agent run`)
- State continuity — `latest_state_for_agent()` reads prior state on session start

### Deferred

- **`vlinder timeline diff`** — compare snapshots at two state commits. The data model supports it; the CLI command can come later.
- **Vector storage versioning** — same three rules, same model. VectorServiceWorker changes mirror ObjectServiceWorker changes. Deferred because the demo only needs KV.
- **In-memory StateStore** — SQLite-only for now. In-memory variant needed when agents declare `memory://` object storage.

### Future Work

- **Garbage collection** — retention policy for historical objects. Append-only means storage grows. Unreachable objects (failed invocations, history beyond a window) need pruning. Separate ADR.
- **Agent-provided storage protocol** — `state_advance(key, content_hash)` and `state_resolve(key)` SDK calls for agents that bring their own Postgres, Qdrant, etc. The platform manages the pointer; the agent manages the data.
- **Multi-agent state merging** — when two agents write to overlapping state, their branches need merge semantics. Same problem as git merge conflicts, same class of solutions.
- **State-aware session viewer** — the existing HTML session viewer could show state hashes per turn and link to snapshot inspection.

## Consequences

- The storage contract is three rules: immutable objects, content addressing, platform-managed pointers
- Every capability (crash safety, isolation, undo, fork, time travel, diff, audit, replay) is a consequence of those rules, not a separately implemented feature
- No transaction methods on the storage trait — the pointer mechanism replaces begin/commit/rollback
- KV and vectors are unified under the same model — both are content-addressed objects
- Agent authors choose their storage backend — the contract is backend-agnostic
- The conversation DAG (git commits) and the state DAG (object model) are two linked timelines with the same structure
- Git's UX vocabulary — checkout, branch, diff, log — maps directly to agent state operations
- Agents that don't care see no change — `kv_put` / `kv_get` work exactly as before
- The only new problem is garbage collection of historical objects
