# System Timeline: How It Works

Two storage systems, both git-shaped, linked at turn boundaries.

## The Two Repos

**Conversation store** (`~/.vlinder/conversations/`): a real git repo.
Each commit is a user message or agent response. `timeline log` and
`timeline fork` operate here.

**State store** (`agents/<name>/data/state.db`) SQLite implementing the
git object model. `kv_put`/`kv_get` data lives here. Explained in detail
below.

They link via the `State:` trailer on agent response commits:

```
commit aaa2222
agent

Added: buy milk

Session: ses-abc12345
Submission: aaa1111
State: sc1              <-- points into the state store
```

## Walkthrough

See [TIMELINE_WALKTHROUGH.md](TIMELINE_WALKTHROUGH.md) for a step-by-step
grocery list example showing the `git log` at every turn, through forking
and switching back.

## Sequence: Normal Run With State Continuity

```
   User              CLI                ConversationStore        StateStore         Agent
    │                 │                       │                      │                │
    │  agent run      │                       │                      │                │
    │────────────────>│                       │                      │                │
    │                 │  git log *_agent_*    │                      │                │
    │                 │──────────────────────>│                      │                │
    │                 │  State: sc1           │                      │                │
    │                 │<─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ │                      │                │
    │                 │                       │                      │                │
    │                 │  set_initial_state(sc1)                      │                │
    │                 │─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─>│                │
    │                 │                       │                      │                │
    │  "add buy milk" │                       │                      │                │
    │────────────────>│                       │                      │                │
    │                 │  commit: user input   │                      │                │
    │                 │──────────────────────>│  (commit aaa1111)    │                │
    │                 │                       │                      │                │
    │                 │  invoke(input, state=sc1)                    │                │
    │                 │─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ >│
    │                 │                       │                      │                │
    │                 │                       │    kv_put(todos,data)│                │
    │                 │                       │< ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─│
    │                 │                       │  value→snap→sc2      │                │
    │                 │                       │─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─>│                │
    │                 │                       │                      │                │
    │                 │                  response: "Added: buy milk" │                │
    │                 │<─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ │
    │                 │                       │                      │                │
    │                 │  commit: agent resp   │                      │                │
    │                 │  + State: sc2         │                      │                │
    │                 │──────────────────────>│  (commit aaa2222)    │                │
    │                 │                       │                      │                │
    │  "Added: ..."   │                       │                      │                │
    │<────────────────│                       │                      │                │
```

On the next `agent run`, the cycle starts again: `git log` finds `State: sc2`
from commit `aaa2222`, and the agent picks up where it left off.

## Sequence: Timeline Fork

```
   User              CLI                ConversationStore
    │                 │                       │
    │  timeline log   │                       │
    │────────────────>│  git log --reverse    │
    │                 │──────────────────────>│
    │                 │  all commits          │
    │                 │<─ ─ ─ ─ ─ ─ ─ ─ ─ ─  ─│
    │  (shows timeline)                       │
    │<────────────────│                       │
    │                 │                       │
    │  timeline fork  │                       │
    │  aaa2222        │                       │
    │────────────────>│                       │
    │                 │  git checkout -b      │
    │                 │  fork-aaa22222 aaa2222│
    │                 │──────────────────────>│
    │                 │                       │  HEAD moves to new branch
    │                 │                       │  at commit aaa2222
    │  "Forked..."    │                       │
    │<────────────────│                       │
    │                 │                       │
    │  agent run      │                       │
    │────────────────>│  git log *_agent_*    │
    │                 │──────────────────────>│  (follows fork branch)
    │                 │  State: sc1           │  (from aaa2222, not later)
    │                 │<─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ │
    │                 │                       │
    │                 │  Agent sees state     │
    │                 │  from the fork point  │
    │                 │                       │
```

After the fork, the conversation store has two branches:

```
main:          ... → aaa2222 → bbb1111 → bbb2222 → bbb3333 → bbb4444
                         │
fork-aaa22222:           └→ (new commits land here)
```

`git log` on each branch only sees its own history. `latest_state_for_agent()`
follows whatever branch HEAD points to; no branch-awareness in Rust code.

## The State Store: Git's Object Model in SQLite

The state store doesn't use git. It uses git's *model*; the same three
object types, the same content-addressing, the same append-only semantics
implemented as three SQLite tables.

### Why not just use git?

Agent state is hot data: many small writes per invocation, read on every
`kv_get`. Git's on-disk format (loose objects, packfiles, index) adds
overhead that makes no sense for this access pattern. SQLite gives us the
same guarantees with better performance for small transactional writes.

The conversation store (one commit per user/agent turn) is a natural fit
for real git. The state store (many writes per turn) is a natural fit for
SQLite with git's model.

### The Three Object Types

Git has blobs, trees, and commits. The state store mirrors this exactly:

```
┌─────────────────────────────────────────────────────────────────┐
│  Git                         State Store                        │
│                                                                 │
│  blob   = raw content    →   Value    = raw bytes (JSON, etc.)  │
│  tree   = path→blob map  →   Snapshot = path→value_hash map     │
│  commit = tree + parent  →   StateCommit = snapshot + parent    │
│  HEAD   = mutable ref    →   State trailer in conversation git  │
└─────────────────────────────────────────────────────────────────┘
```

Each object is identified by its SHA-256 hash. Same content = same hash.
Writes are `INSERT OR IGNORE` — storing the same object twice is a no-op.

### The Three Tables

```sql
state_values (
    hash    TEXT PRIMARY KEY,   -- SHA256(content)
    content BLOB NOT NULL
)

state_snapshots (
    hash    TEXT PRIMARY KEY,   -- SHA256(sorted JSON of entries)
    entries TEXT NOT NULL        -- {"path": "value_hash", ...}
)

state_commits (
    hash          TEXT PRIMARY KEY,   -- SHA256(snapshot_hash + ":" + parent_hash)
    snapshot_hash TEXT NOT NULL,
    parent_hash   TEXT NOT NULL       -- "" for root
)
```

### How a Write Works

When an agent calls `kv_put("/todos.json", '["buy milk"]')`:

```
Step 1: Hash the content
   content = '["buy milk"]'
   value_hash = SHA256(content) = "a1b2c3..."

Step 2: Store the value (idempotent)
   INSERT OR IGNORE INTO state_values (hash, content)
   VALUES ("a1b2c3...", '["buy milk"]')

Step 3: Clone parent snapshot, update the entry
   parent snapshot: {"/config.json" → "x1y2z3..."}
   new snapshot:    {"/config.json" → "x1y2z3...", "/todos.json" → "a1b2c3..."}
   snapshot_hash = SHA256(sorted JSON) = "d4e5f6..."

Step 4: Store the snapshot (idempotent)
   INSERT OR IGNORE INTO state_snapshots (hash, entries)
   VALUES ("d4e5f6...", '{"config.json":"x1y2z3...","todos.json":"a1b2c3..."}')

Step 5: Create state commit
   commit_hash = SHA256("d4e5f6..." + ":" + parent_commit_hash) = "sc2..."

Step 6: Store the state commit (idempotent)
   INSERT OR IGNORE INTO state_commits (hash, snapshot_hash, parent_hash)
   VALUES ("sc2...", "d4e5f6...", "sc1...")

Step 7: Return "sc2..." to the caller
```

The caller (ObjectServiceWorker) returns this hash to the harness, which
threads it through subsequent operations and eventually records it as the
`State:` trailer on the agent response commit.

### How a Read Works

When an agent calls `kv_get("/todos.json")` with state `sc2`:

```
Step 1: Load state commit
   SELECT snapshot_hash FROM state_commits WHERE hash = "sc2..."
   → "d4e5f6..."

Step 2: Load snapshot
   SELECT entries FROM state_snapshots WHERE hash = "d4e5f6..."
   → {"/config.json" → "x1y2z3...", "/todos.json" → "a1b2c3..."}

Step 3: Look up path
   "/todos.json" → "a1b2c3..."

Step 4: Load value
   SELECT content FROM state_values WHERE hash = "a1b2c3..."
   → '["buy milk"]'
```

Three hops: commit → snapshot → value. Each hop is a primary key lookup.

### How Writes Chain

Multiple writes in one invocation form a chain:

```
kv_put("/todos.json", v1)     state="" (root)
   → value v1, snapshot s1{todos→v1}, commit sc1(s1, "")

kv_put("/config.json", v2)    state="sc1"
   → value v2, snapshot s2{todos→v1, config→v2}, commit sc2(s2, sc1)

kv_put("/todos.json", v3)     state="sc2"
   → value v3, snapshot s3{todos→v3, config→v2}, commit sc3(s3, sc2)
```

Each write creates a new snapshot that includes all paths from the parent
plus the changed one. Old values and snapshots remain in the database —
they're never deleted, only shadowed by newer commits.

```
sc1 → s1 → {todos→v1}
 │
sc2 → s2 → {todos→v1, config→v2}
 │
sc3 → s3 → {todos→v3, config→v2}

v1, v2, v3 all still exist. s1, s2, s3 all still exist.
Only the pointer (which state commit the platform uses) changes.
```

### Why This Matters for Forking

The state store is shared across timelines. When you fork:

```
main branch:          ... → agent(State: sc3) → ...
fork branch:          ... → agent(State: sc1) → ...
```

Both branches reference hashes in the *same* `state.db`. The fork branch
reads from `sc1`, which resolves to snapshot `s1`, which only has `todos→v1`.
The main branch reads from `sc3`, which resolves to `s3`, which has both
updated todos and config.

No data is copied. No state is duplicated. The fork just reads from a
different point in the same append-only store.

## Key Insight

The conversation store is a real git repo. Branches, log, and checkout
are not reimplemented, they are actual git operations. The platform
delegates timeline semantics to git rather than building its own.

The state store uses git's *model* (content-addressed objects, snapshots,
commits) but not git's *tool*, SQLite is a better fit for the access
pattern. The two systems link at turn boundaries via the `State:` trailer.
