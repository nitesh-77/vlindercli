# ADR 108: GitFS Mount as Unified DAG Store Interface

**Status:** Draft

## Context

The conversation store is a real git repo (`~/.vlinder/conversations/`).
The state store uses SQLite with git's object model. Both contain the
data developers and agents need most: what happened during an agent run,
what state it produced, and how to debug failures.

But accessing this data requires git plumbing. Reading a conversation
means traversing commits. Inspecting state means querying SQLite with
knowledge of the object model. AI coding tools connected via MCP can't
do either — they read files. Agents running in containers can't do
either — they read files. Humans debugging at the terminal reach for
`ls` and `cat`, not `git cat-file`.

The most valuable capability in ADR 107 — making runtime state queryable
through the MCP server — is impossible without bridging this gap. The
data is there. The interface is wrong.

## Decision

### GitFS FUSE mount

`vlinderd` manages a FUSE mount at `~/.vlinder/mnt/` that projects the
git-based DAG as a navigable filesystem. The mount is not a read-only
view — it is the **primary interface** to the DAG store. The
GitDagStore writes through the mount, and everything reads from it.

```
                writes                     reads
GitDagStore ──────→ ┌─────────┐ ←────── MCP tools
                    │  gitfs  │ ←────── agents
                    │  mount  │ ←────── humans (ls, cat)
                    └────┬────┘ ←────── support fleet
                         │
                    storage engine
                  (git / SQLite / whatever)
```

One interface for both sides. The storage engine behind the FUSE layer
is an implementation detail.

### Mount structure

```
~/.vlinder/mnt/
  conversations/
    <agent-run>/
      latest/
        payload
        state
      history/
        <commit-sha>/
          payload
          state
          parent → ../../../<parent-sha>/
  ```

Git history becomes directories. Commits become folders with symlinked
parents. Time-travel is directory traversal.

### Lifecycle

`vlinderd` owns the mount. Start the daemon, the mount appears. Stop
it, it unmounts cleanly. The mount lifecycle follows the daemon
lifecycle — no separate process to manage.

### Implications for the container contract

Agents that need to inspect their own conversation history — or fork
from a previous state — read and write files at a mount path. No SDK,
no client library, no git knowledge. The container contract gets
simpler: "your state is at this path."

### Implications for ADR 107

The MCP server's most valuable capability — querying runtime state —
becomes trivial. The support fleet's runtime-state specialist agent
reads files from the mount. Any MCP tool reads files from the mount.
Increment 3 of the implementation plan (which was "totally impossible"
without this) becomes a file-reading exercise.

### Storage engine pluggability

The mount is the abstraction boundary. The engine behind it can change
without affecting any consumer:

- Real git repo (current)
- SQLite with git's object model (current state store)
- Something else entirely

Nothing above the mount cares.

## Consequences

- `vlinderd` gains FUSE mount management — new dependency on a FUSE
  library (e.g. fuser crate on Linux/macOS)
- The GitDagStore writes through the mount instead of directly to git —
  this changes the write path
- Agents, MCP tools, support fleet, and humans all use the same
  interface — no parallel access patterns to maintain
- The container contract simplifies: state access is file I/O at a
  known path
- Platform requires FUSE support on the host OS — this is standard on
  Linux and macOS but may need macFUSE on macOS
- Unblocks ADR 107 increment 3: runtime state becomes queryable through
  MCP without custom git plumbing
