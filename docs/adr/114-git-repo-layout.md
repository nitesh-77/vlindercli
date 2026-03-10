# ADR 114: Git Repo Layout — Agent/Session Folders with Timeline Indexes

## Status

Proposed

## Context

ADR 068 defined the git conversations repo as a flat accumulated tree with
all message directories at the root. ADR 065 established git as a projection
of the canonical SQL DAG — a human-friendly view, not the source of truth.

Two problems emerged during use:

1. **Navigation is broken.** Sessions are orphan commit chains under
   `refs/sessions/<id>`. `git log` on main shows nothing. `git checkout main`
   doesn't go back to "current." Users must know internal ref names to browse.

2. **Forking is unclear.** With orphan chains, repair branching works per-session.
   But users can't use standard git commands (`git branch`, `git merge`) because
   the orphan model doesn't map to how people think about git.

The root cause: the repo structure doesn't match what people expect from a git
repo. Orphan chains and flat trees require platform-specific knowledge to
navigate. The projection should be self-explanatory.

## Decision

**All commits go on `main`. Message directories nest under
`<agent>/<session>/`. Each session has a `timelines/` folder with one file per
timeline and an `ACTIVE` marker.**

### Folder structure

```
todoapp/
  ses-9a22f767/
    timelines/
      main            ← default timeline
      ACTIVE          ← "main"
    001-invoke/
    002-request-kv/
    003-response-kv/
    004-request-infer/
    005-response-infer/
    006-complete/
  ses-abc12345/
    timelines/
      main
      ACTIVE
    001-invoke/
    ...
agent.toml
platform.toml
models/
```

Agent name is the top-level folder. Session ID is the second level. Message
directories use a sequence number prefix instead of timestamps for readability.
`ls` gives natural ordering.

### Timeline files

Each timeline is a file in `timelines/` listing the message directories that
belong to that path, one per line:

```
timelines/main:
001-invoke
002-request-kv
003-response-kv
```

The `ACTIVE` file contains the name of the current timeline (e.g. `main`).
New messages append to whichever timeline ACTIVE points to.

### Fork

`vlinder session fork ses-xyz --from 002 --name retry-1` creates a new
timeline file copied from `main` up to the fork point:

```
timelines/main:       001-invoke, 002-request-kv, 003-response-fail
timelines/retry-1:    001-invoke, 002-request-kv
ACTIVE:               retry-1
```

### Repair

Repair agent runs. New messages append to the active timeline:

```
timelines/main:       001-invoke, 002-request-kv, 003-response-fail
timelines/retry-1:    001-invoke, 002-request-kv, 004-response-repair, 005-complete
ACTIVE:               retry-1
```

All message directories stay in the tree — nothing is deleted.

### Promote

Promote is implicit: ACTIVE already points to the winning timeline. The old
timeline file stays as the record of what failed.

`diff timelines/main timelines/retry-1` shows exactly what diverged.

### No git branches for repair

Everything stays on `main`. No git branches, no merge conflicts. SQL drives
the fork/repair/promote logic. The `timelines/` folder is the git-side
projection of SQL timeline rows.

`git log -p -- todoapp/ses-xyz/timelines/` tells the complete story of every
fork, repair, and promote for that session.

### No orphan chains

All sessions commit to `main`. No `refs/sessions/` refs. Standard git commands
work:

- `git log` — full history across all agents and sessions
- `git log -- todoapp/` — history for one agent
- `git log -- todoapp/ses-xyz/` — history for one session
- `git checkout main` — back to latest
- `git diff HEAD~1` — what the last message added

### Metadata at agent level

`agent.toml`, `platform.toml`, and `models/` move under the agent folder (or
stay at root — one per agent if multiple agents run). Updated on each commit
with the latest registry state.

### Relationship to SQL

SQL DAG store is canonical. Git is a projection (ADR 065). Timeline files
mirror SQL timeline rows. ACTIVE mirrors which timeline is current. SQL drives
fork/repair/promote logic. Git makes it browsable and diffable.

### CLI evolution

`vlinder session` becomes the primary interface for session inspection, fork,
repair, and promote — querying SQL directly. `vlinder timeline` (git
passthrough, ADR 068) is kept as an escape hatch until `session` covers all
use cases, then deprecated.

## Consequences

- `git log`, `git checkout main`, `git diff` all work as expected
- Session isolation is at the folder level, not the commit graph level
- Multiple timelines per session — all preserved, none overwritten
- `diff timelines/a timelines/b` compares any two paths
- `git log -p -- timelines/` shows the full fork/repair history
- No platform-specific ref naming to learn
- No git branches needed for repair — linear history on main
- Supersedes the orphan-chain approach from the sessions-as-islands implementation
- `vlinder timeline` kept as git passthrough until `vlinder session` is complete
