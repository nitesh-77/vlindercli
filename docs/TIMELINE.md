# Sessions and State: How It Works

Two storage systems, both content-addressed, linked at turn boundaries.

## The Two Stores

**Merkle DAG** (SQL database): the primary store. Every side effect
(invocations, service requests, responses, completions) is a
content-addressed node in a Merkle chain. `session list`, `session fork`,
and `session promote` operate here.

**Conversations repo** (`~/.vlinder/conversations/`): a read-only git
projection of the DAG. Useful for inspection with standard git tools.

**State store** (`agents/<name>/data/state.db`): SQL database implementing
git's object model. `kv_put`/`kv_get` data lives here.

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
grocery list example showing state at every turn, through forking
and switching back.

## Sequence: Normal Run With State Continuity

```
   User              CLI                DAG / ConvStore          StateStore         Agent
    │                 │                       │                      │                │
    │  agent run      │                       │                      │                │
    │────────────────>│                       │                      │                │
    │                 │  find latest state    │                      │                │
    │                 │──────────────────────>│                      │                │
    │                 │  State: sc1           │                      │                │
    │                 │<─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ │                      │                │
    │                 │                       │                      │                │
    │                 │  set_initial_state(sc1)                      │                │
    │                 │─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─>│                │
    │                 │                       │                      │                │
    │  "add buy milk" │                       │                      │                │
    │────────────────>│                       │                      │                │
    │                 │  record: user input   │                      │                │
    │                 │──────────────────────>│                      │                │
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
    │                 │  record: agent resp   │                      │                │
    │                 │  + State: sc2         │                      │                │
    │                 │──────────────────────>│                      │                │
    │                 │                       │                      │                │
    │  "Added: ..."   │                       │                      │                │
    │<────────────────│                       │                      │                │
```

On the next `agent run`, the cycle starts again: the platform finds
`State: sc2` and the agent picks up where it left off.

## Sequence: Session Fork

```
   User              CLI                Session Store
    │                 │                       │
    │  session list   │                       │
    │────────────────>│  query sessions       │
    │                 │──────────────────────>│
    │                 │  session history      │
    │                 │<─ ─ ─ ─ ─ ─ ─ ─ ─ ─  ─│
    │  (shows turns)  │                       │
    │<────────────────│                       │
    │                 │                       │
    │  session fork   │                       │
    │  ses-abc --from │                       │
    │  sc2 --name fix │                       │
    │────────────────>│                       │
    │                 │  create branch "fix"  │
    │                 │  from state sc2       │
    │                 │──────────────────────>│
    │                 │                       │
    │  "Forked..."    │                       │
    │<────────────────│                       │
    │                 │                       │
    │  agent run      │                       │
    │  todoapp        │                       │
    │  --branch fix   │                       │
    │────────────────>│  find state for "fix" │
    │                 │──────────────────────>│  (follows fix branch)
    │                 │  State: sc2           │  (not sc3)
    │                 │<─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ │
    │                 │                       │
    │                 │  Agent sees state     │
    │                 │  from the fork point  │
    │                 │                       │
```

After the fork, the session has two branches:

```
main:       ... → sc1 → sc2 → sc3 → sc4
                          │
fix:                      └→ (new state commits land here)
```

Both branches share the same state store. The fork just reads from
a different point in the same append-only store. No data is copied.

## The State Store: Git's Object Model in SQL

The state store uses git's *model* — the same three object types,
the same content-addressing, the same append-only semantics —
implemented as SQL tables.

### The Three Object Types

Git has blobs, trees, and commits. The state store mirrors this exactly:

```
┌─────────────────────────────────────────────────────────────────┐
│  Git                         State Store                        │
│                                                                 │
│  blob   = raw content    →   Value    = raw bytes (JSON, etc.)  │
│  tree   = path→blob map  →   Snapshot = path→value_hash map     │
│  commit = tree + parent  →   StateCommit = snapshot + parent    │
│  HEAD   = mutable ref    →   State ref in DAG node              │
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

### Why This Matters for Forking

The state store is shared across branches. When you fork:

```
main branch:          ... → agent(State: sc3) → ...
fix branch:           ... → agent(State: sc1) → ...
```

Both branches reference hashes in the *same* state database. The fix branch
reads from `sc1`, which resolves to snapshot `s1`, which only has `todos→v1`.
The main branch reads from `sc3`, which resolves to `s3`, which has both
updated todos and config.

No data is copied. No state is duplicated. The fork just reads from a
different point in the same append-only store.

## Key Insight

The primary store is a content-addressed Merkle DAG in a SQL database.
The conversations repo is a read-only git projection — useful for
inspection with standard git tools, but not a system component.

The state store uses git's *model* (content-addressed objects, snapshots,
commits) but not git's *tool* — SQL is a better fit for the access
pattern. The two systems link at turn boundaries via the `State:` ref.
