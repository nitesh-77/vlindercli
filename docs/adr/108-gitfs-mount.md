# ADR 108: Git Repos as Universal Agent Storage via FUSE Mounts

**Status:** Draft

## Context

Git gives you content addressing, branching, merging, full history, and
distributed replication — proven infrastructure. Vlinder's conversation
store already uses a real git repo. The state store uses SQLite with
git's object model. The platform tagline is "AI agents that can time
travel."

But agents can't use any of this. They run in containers. They do file
I/O. They don't speak git. The conversation store requires git plumbing
to read. The state store requires SQLite queries with knowledge of the
object model. AI coding tools connected via MCP can't do either — they
read files. Humans reach for `ls` and `cat`, not `git cat-file`.

The most valuable capability in ADR 107 — making runtime state queryable
through the MCP server — is impossible without bridging this gap. But
the gap is bigger than one use case. Every agent that wants versioned,
forkable, auditable storage faces the same problem: git is the right
backend, but the interface is wrong.

## Decision

### Git repos as a storage primitive

Any agent can declare git repos as storage in its manifest. The platform
mounts them into the container via FUSE. The agent reads and writes
files. Behind the mount, it's git — every write is versioned, history is
navigable, forking is branching. The agent doesn't know or care.

```toml
[requirements.mounts]
my-data  = { git = "local", path = "/data" }
shared   = { git = "https://github.com/team/shared-state.git", path = "/shared" }
codebase = { git = "https://github.com/org/repo.git", ref = "main", path = "/code" }
```

- `local` — platform creates and manages a git repo for the agent
- Remote URL — platform clones/pulls and mounts
- `ref` — pin to a branch, tag, or commit
- `path` — where it appears inside the container

### What the agent sees

```
/data/
  current/            # working tree — read and write here
    file-a.txt
    file-b.json
  history/
    <commit-sha>/
      file-a.txt
      file-b.json
      parent → ../../<parent-sha>/
```

`current/` is the working tree. Writes here create new versions.
`history/` exposes the full commit graph as directories. Time travel
is directory traversal. Fork is a branch — a new `current/` that
shares history with the original.

### What the platform manages

`vlinderd` owns the FUSE mounts. For each declared mount:

1. Create or clone the git repo
2. Mount via FUSE into the container's filesystem
3. Intercept writes, commit to git
4. Serve reads from git objects
5. Unmount on container stop

The mount lifecycle follows the container lifecycle.

### The conversation store is just one mount

The platform's own conversation store (`~/.vlinder/conversations/`) is
demoted from special-case infrastructure to one git repo among many.
It's mounted the same way, accessed the same way, versioned the same
way. The platform uses the same primitive it offers to every agent.

```
                writes                     reads
agents ───────────→ ┌─────────┐ ←────── MCP tools
GitDagStore ──────→ │  gitfs  │ ←────── other agents
                    │  mount  │ ←────── humans (ls, cat)
                    └────┬────┘ ←────── support fleet
                         │
                    git repo
```

### Collaboration through shared mounts

Two agents that declare the same git repo share state through files.
Git handles concurrent writes, conflict resolution, and provides a
full audit trail. The platform doesn't need a separate collaboration
protocol — git is the protocol.

### What "time travel" means now

It's not a platform feature bolted onto agent runs. It's a property
of the storage primitive. Any data an agent writes to a git-mounted
path gets time travel automatically. The tagline "AI agents that can
time travel" applies to any data the agent touches, not just the
conversation store.

## Consequences

- `vlinderd` gains FUSE mount management — new dependency on a FUSE
  library (e.g. fuser crate on Linux/macOS)
- `agent.toml` gains `[requirements.mounts]` — new manifest schema
- The GitDagStore writes through the mount instead of directly to git
- Platform requires FUSE support on the host (standard on Linux,
  macFUSE on macOS)
- The conversation store loses its special status — it's one mount
  among many, accessed through the same interface
- Agents get versioned, forkable, auditable storage through file I/O —
  no SDK, no client library
- Agent collaboration becomes shared git repos — no new protocol needed
- Unblocks ADR 107 increment 3: runtime state is queryable through MCP
  as file reads
- "Time travel" becomes a property of storage, not a platform feature
