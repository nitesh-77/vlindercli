# ADR 061: Runtime-Captured DAG

**Status:** Superseded by ADR 067

**Supersedes:** ADR 055 (partially — the requirement that agents use platform storage for time travel to work)

## Context

ADR 055 built version-controlled agent state using git's object model: immutable objects, content addressing, platform-managed pointers. It works. But it requires agents to use the platform's KV and vector bridges for the DAG to capture their state. Agents that bring their own database or write to local files are invisible to the timeline.

This is backwards. The platform already sees every payload at agent boundaries — every NATS message in and every response out. That stream *is* the causal graph. We don't need agents to cooperate. The runtime can build the DAG from what it already observes.

## Decision

**The runtime captures agent boundary messages from the NATS stream and indexes them into a SQL database.** Agents don't need to use platform storage, report hashes, or follow any content-addressing protocol. The platform builds the Merkle DAG from observed traffic alone.

### What the runtime captures

At every agent invocation boundary:

| Field | Source |
|-------|--------|
| Payload in | NATS message delivered to agent |
| Response out | NATS message returned by agent |
| Agent name | NATS subject |
| Session ID | Message header |
| Timestamp | Message metadata |
| Parent node | The node that produced this invocation's input |

Each node is content-addressed: `sha256(payload_in || response_out || parent_hash)`. The parent hash links nodes into a DAG.

### How it works

```
NATS stream (already exists)
    │
    ▼
┌──────────────┐
│  DAG worker  │  tails stream, computes hashes, writes nodes
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  SQL store   │  nodes table, indexed for ancestry queries
└──────────────┘
```

A dedicated worker (same pattern as existing service workers) tails the NATS stream. For every request/response pair at an agent boundary, it:

1. Computes content hash of the pair
2. Links to the parent node (the previous step in this session's causal chain)
3. Writes the node to the database

### Schema (minimal)

```sql
CREATE TABLE dag_nodes (
    hash        CHAR(64) PRIMARY KEY,   -- sha256
    parent_hash CHAR(64) REFERENCES dag_nodes(hash),
    agent       TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    payload_in  TEXT NOT NULL,           -- the message the agent received
    payload_out TEXT NOT NULL,           -- the message the agent returned
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_dag_session ON dag_nodes(session_id, created_at);
CREATE INDEX idx_dag_agent ON dag_nodes(agent);
CREATE INDEX idx_dag_parent ON dag_nodes(parent_hash);
```

### What falls out

| Capability | Type | Agent cooperation | How |
|------------|------|-------------------|-----|
| **Timeline log** | Analytical | None | `SELECT * FROM dag_nodes WHERE session_id = ? ORDER BY created_at` |
| **Diff** | Analytical | None | Compare `payload_out` at two nodes |
| **Bisect** | Analytical | None | Binary search the node chain for where output diverged |
| **Replay** | Analytical | None | Feed historical `payload_in` back through the agent |
| **Audit** | Analytical | None | Walk parent_hash links — full causal chain |
| **Fork** | Operational | Pure function or checkpoint/restore | Git branch for conversation + `state_ref` for agent state recovery |

### What happens to state_store.rs

The existing `StateStore` (content-addressed SQLite with snapshots and commits) becomes unnecessary for the DAG. The DAG is built from observed traffic, not from agent-reported state.

Agent KV (`ObjectServiceWorker`) becomes plain KV — no hashing, no snapshots, no commit chains. Agents that want KV get simple key-value storage. The platform doesn't need it to be content-addressed anymore.

### Analytical vs operational: what's free, what's taxed

The DAG is a downstream projection of the NATS stream. It's read-only. This splits capabilities into two categories:

**Free (analytical — read-only DAG queries, no agent cooperation):**

- Diff: compare payloads at two nodes
- Bisect: binary search the chain for where output diverged
- Replay: feed historical `payload_in` back through the agent
- Audit: walk the full causal chain

Every agent gets these. Zero effort. The platform observes, the agent doesn't participate.

**Taxed (operational — fork changes what happens next):**

Fork rolls back the timeline. The git conversation repo branches. But the agent's internal state doesn't roll back — it's still at the latest point. Pure function agents (all state in the message) are unaffected. Agents with internal state see stale data after fork.

Fork correctness is proportional to agent purity.

### Checkpoint/restore bridge protocol

Agents that want correct fork behavior without being pure functions can opt into checkpoint/restore. Two optional signals on the existing bridge:

```
Platform → Agent (on invocation start):  { "state_ref": "<dag_node_hash>" }
Platform → Agent (after response):       { "checkpoint": "<dag_node_hash>" }
```

**Normal flow:** `state_ref` is the previous turn's DAG node hash. The agent can ignore it. After the agent responds, the platform sends `checkpoint` with the new node hash. The agent snapshots its internal state, tagged with that hash.

**Fork flow:** `state_ref` is the fork point's node hash. The agent restores the snapshot tagged with that hash. Everything from that point forward is a new chain.

Agents that don't implement checkpoint/restore ignore the signals — backwards compatible. They get diff but not fork. Agents that implement it get both.

The agent chooses its own snapshot mechanism — pg_dump, SQLite backup, filesystem copy, whatever. The platform doesn't care how. It only provides the hash to tag/restore by.

### Agent author tax

| Tier | What the agent does | What they get |
|------|-------------------|---------------|
| **Zero effort** | Nothing. Use any storage, any DB. | Diff, bisect, replay, audit — all analytical operations |
| **Pure functions** | All state in the message, no internal storage | Everything above + fork works correctly (nothing to roll back) |
| **Checkpoint/restore** | Implement the two bridge signals, snapshot internal state by hash | Everything above + fork works correctly (platform restores by hash) |

The platform incentivises pure function agents: the less internal state you hide, the more the DAG sees, and the less work you do for fork. But it doesn't punish impure agents — they still get all analytical operations for free. Checkpoint/restore is the escape hatch for stateful agents that need fork.

### Relationship to the conversation git repo (ADR 054)

The conversation git repo and the DAG store serve different roles. They are not redundant.

**The git repo is the operational store.** It's in the write path — the harness commits user input and agent responses synchronously during the conversation. It determines what happens next: `latest_state_for_agent` scans git log to find prior state, `fork_at` creates a git branch. The REPL, the harness, and the agent lifecycle all read from git.

**The DAG store is the analytical store.** It's a disconnected downstream projection — a worker tails the NATS stream asynchronously and indexes into a SQL database. It has no role in the conversation flow. Nothing reads from it during agent execution. It exists purely for after-the-fact inspection: diff, bisect, audit.

They overlap in content (both capture user inputs and agent responses) but diverge in scope and purpose:

| | Git repo | DAG store |
|-|----------|-------------|
| **Path** | Synchronous write path | Async downstream projection |
| **Scope** | User ↔ agent conversation text | All NATS traffic including inter-agent messages in fleets |
| **Fork** | `git checkout -b` — operational, changes what agent sees next | Not involved — fork is a git operation |
| **Diff/bisect** | Poor fit (git diff on JSON session files) | Purpose-built (indexed, queryable, content-addressed) |
| **Failure mode** | Blocks conversation if git fails | Conversation continues; analytical features degrade |

**Future:** The git repo may eventually be replaced by a database-backed operational store, collapsing both into one system. But that's a separate decision. For now, git stays because it works, it's in the hot path, and replacing it is a larger change than introducing the DAG.

### Scaling

One database instance per tenant cell. Scaling = more cells, not bigger instances. The DAG is append-only and partitioned by session, so cells are independent.

## Consequences

- The DAG is built from observed NATS traffic — no agent cooperation required
- Analytical operations (diff, bisect, replay, audit) are free for all agents
- Fork correctness requires either pure function design or checkpoint/restore — the platform makes the cost of impurity visible at fork time, not before
- `state_store.rs` and its content-addressed machinery are no longer needed for the DAG
- Agent KV simplifies to plain KV (no hashing, snapshots, or commit chains)
- Agents can bring any storage they want — the platform still captures their boundaries
- The DAG store requires any SQL database with recursive CTE support (SQLite for local dev, Postgres for distributed) — rich queries for ancestry, diff, fork, bisect
- The conversation git repo (ADR 054) stays as the operational store — synchronous, in the write path, drives fork. The DAG store is the analytical store — async, disconnected, drives diff/bisect/audit. They coexist; collapsing them is a future decision.
- The bridge protocol gains two optional fields (`state_ref`, `checkpoint`) — backwards compatible, no existing agents break
