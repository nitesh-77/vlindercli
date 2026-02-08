# ADR 055: Transactional Storage

**Status:** Proposed

## Context

ADR 054 made `SubmissionId` = git commit SHA. Every `RequestMessage` carries a SHA representing a position in a causal graph. Storage engines receive this on every operation but ignore it — they overwrite state on each write.

This means storage is destructive and non-isolated. If an agent crashes mid-invocation after 23 of 40 writes, the first 22 writes are permanently applied — half-finished work is visible. If two agents write to overlapping keys, they interleave unpredictably. There is no rollback, no isolation, no atomicity.

The fix isn't versioning — it's transactions. An agent's invocation is a **work unit**. Its mutations should be invisible until the work unit completes, and discardable if it fails. Time travel is a consequence of this contract, not a goal in itself.

## Decision

### The Contract

Each agent invocation is a **work unit** with transactional semantics:

**1. Begin.** When an invoke starts, the platform provides a parent SHA (the SubmissionId). This is the branch point — the state the agent inherits.

**2. Mutate.** Every write (kv-put, vector-store, etc.) is a mutation within the work unit. Each mutation produces a new SHA, chained from the previous:

```
incoming SHA: aaa

op1: kv_put("/todos.json", data1)         → sha1 = hash(aaa  + op1) = bbb
op2: embed("chunk-0", vec0, "intro")      → sha2 = hash(bbb  + op2) = ccc
op3: kv_put("/todos.json", data2)         → sha3 = hash(ccc  + op3) = ddd
...
op40: kv_put("/metadata.json", meta)      → sha40 = hash(...) = zzz
```

The SHA is deterministic: `hash(parent_sha + op_type + key + content)`. Same inputs always produce the same chain.

**3. Commit.** When the agent returns successfully, all mutations become visible atomically. The final SHA (`zzz`) is the work unit's state identifier.

**4. Rollback.** If the agent crashes or errors, all mutations are discarded. State reverts to the parent SHA. The next retry starts a fresh work unit from the same parent.

### Isolation Guarantees

- **Read-your-own-writes.** Within a work unit, the agent sees its own mutations. After `kv_put("/foo", v2)`, a subsequent `kv_get("/foo")` returns `v2`.
- **Snapshot isolation from other work units.** Other agents (or other sessions) see the state as of the last committed work unit, not in-progress mutations.
- **Atomic visibility.** On commit, all mutations become visible at once. There is no window where partial state is observable.

### The SHA Chain

Reads do not advance the chain. Only mutations do. The chain is linear within a work unit:

```
aaa → bbb → ccc → ddd → ... → zzz
 ↑                               ↑
 branch point                    commit point
 (parent SHA)                    (final state SHA)
```

The platform computes the SHA. The agent receives it as the return value of each write:

```python
sha1 = kv_put("/todos.json", content)    # returns "bbb..."
sha2 = kv_put("/config.json", config)    # returns "ccc..."
```

Agents can ignore the return value (backwards compatible — it's just a string instead of `"ok"`). Agents that want fine-grained history can store and reference these SHAs.

### Cross-Turn Continuity

The conversation DAG (git) is the "macro" timeline. The SHA chain is the "micro" timeline within each turn. They link at the branch points:

```
conversation:  ── user1 ──── agent1 ──── user2 ──── agent2 ──
                    │           ↑           │           ↑
                    │        commit         │        commit
                    ▼           │           ▼           │
work units:         └─ a→b→...→z           └─ d→e→...→y
```

Each work unit's parent SHA is the conversation commit. The committed state SHA is recorded on the agent response commit (as a `State` trailer):

```
agent

This article discusses...

Session: ses-abc123
Submission: aaa
State: zzz
```

To resolve "latest state at the start of turn 2": read the `State` trailer from the previous agent response commit. This links the two DAGs without requiring storage engines to understand git.

### Where the SHA is Computed

The platform computes the chain. Specifically, the `ServiceRouter` (which dispatches bridge calls to storage workers) maintains the current chain position:

1. Invoke starts → chain initialized to SubmissionId
2. Agent calls `kv_put` → bridge computes `new_sha = hash(chain + payload)`, sends request tagged with `new_sha` to storage worker, advances chain
3. Storage worker stores the mutation tagged with the SHA
4. Bridge returns `new_sha` to agent
5. Repeat for each mutation
6. Invoke completes → final chain SHA is the committed state

Agents that bring their own storage and want to participate in the chain implement the same hash function. The platform publishes the algorithm.

## Implementation Levels

The contract enables incremental implementation. Each level builds on the previous:

### Level 1: Isolation (crash safety)

Storage engines implement `begin` / `commit` / `rollback`. Mutations are buffered and applied atomically on commit. Discarded on rollback. No SHA chain, no versioning. Just correctness.

- **Storage trait**: add `begin_work_unit(parent: &str)`, `commit_work_unit()`, `rollback_work_unit()`
- **Workers**: call `begin` when receiving the first request for a submission, `commit` when invoke completes, `rollback` on timeout/error
- **Agent change**: none

### Level 2: SHA chain (audit log)

Each mutation returns a SHA. The chain is recorded. Provides a complete operation log for debugging: "what did the agent actually do?"

- **ServiceRouter**: maintain chain state, compute SHAs, return them to agents
- **Storage engines**: tag each write with its SHA
- **Agent change**: `kv_put` returns a SHA string instead of `"ok"` (backwards compatible)

### Level 3: Historical reads (opt-in time travel)

Agents can read state at any historical SHA. New SDK operation: `kv_get_at(path, sha)`.

- **Storage trait**: add `get_file_at(path: &str, version: &str)`
- **SdkMessage**: add `KvGetAt` variant
- **Bridge**: add `/kv/get-at` endpoint
- **Agent change**: opt-in — agents that want time travel use the new endpoint

### Level 4: Scoped reads (transparent time travel)

The platform scopes all reads to the current work unit's lineage. The agent calls `kv_get(path)` and gets the value from its lineage, not the global latest. Agents are fully isolated without knowing it.

- **Read resolution**: platform provides ancestor list (from `State` trailers on conversation commits), storage queries `WHERE sha IN (ancestors)`
- **Agent change**: none — reads are transparently scoped

## Backend Strategy

The contract is backend-agnostic. Each storage engine implements isolation and the SHA chain using its native mechanisms:

| Backend | Isolation | SHA chain |
|---------|-----------|-----------|
| SQLite | `BEGIN` / `COMMIT` / `ROLLBACK` (native) | `version` column, INSERT not UPSERT |
| Postgres | Same (native transactions) | Same |
| Redis | `MULTI` / `EXEC` / `DISCARD` | Composite key `{path}:{sha}` |
| S3 | Write to staging prefix, move on commit | Version tag in object metadata |
| In-memory | Copy-on-write HashMap | Version-keyed entries |

Agents that bring their own storage implement the contract however they choose. The platform doesn't impose a mechanism — just the semantics.

## Consequences

- Agent invocations gain crash safety — partial writes are never visible
- The SHA chain provides a complete per-operation audit log for free
- Time travel falls out of keeping mutations instead of discarding them
- Replay = create a new work unit from an old parent SHA
- Branching = two work units from the same parent SHA
- Each level is independently useful — Level 1 alone is a significant improvement
- Storage growth is bounded by GC policy (future ADR): prune committed work units older than N turns
- The conversation DAG (git) and the operation chain (SHA) are two linked timelines: macro (turns) and micro (individual writes)
- Agents that don't care about any of this see no change — `kv_put` / `kv_get` work exactly as before
