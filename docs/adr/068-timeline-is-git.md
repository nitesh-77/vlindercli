# ADR 068: Timeline Is Git

## Status

Accepted

## Context

With ADR 064 (git as agent protocol) and ADR 067 (one message, one node), every agent interaction is a git commit in `~/.vlinder/conversations/`. The conversation IS the git graph.

The current `vlinder timeline` command reimplements what git already does: it parses `git log` output, extracts trailers, formats entries, and renders them. It's a custom viewer for data that git already knows how to display.

If every message is a commit with a human-readable message (`invoke: cli → support-agent`), then `git log` IS the timeline viewer. `git diff` IS the diff tool. `git show` IS the node inspector. `git bisect` IS the debugger.

There's nothing to reimplement. Just point git at the right repo.

## Decision

**`vlinder timeline` passes its arguments to git, operating on `~/.vlinder/conversations/`.** It is `git -C ~/.vlinder/conversations/` with a shorter name.

```
vlinder timeline log            →  git -C ~/.vlinder/conversations/ log
vlinder timeline show abc123    →  git -C ~/.vlinder/conversations/ show abc123
vlinder timeline diff a1 b2     →  git -C ~/.vlinder/conversations/ diff a1 b2
vlinder timeline branch         →  git -C ~/.vlinder/conversations/ branch
vlinder timeline bisect start   →  git -C ~/.vlinder/conversations/ bisect start
```

Any git subcommand works. The user brings their own git knowledge. The platform adds nothing — and removes nothing.

### Convenience aliases

`vlinder timeline` can provide shorthand for common patterns without replacing git:

```
vlinder timeline route <session_id>
```

This is sugar for `git log --oneline --grep="Session: <session_id>"` — the Route domain model (ADR 063) formats the output. It's the one command that adds domain-specific value on top of git.

Everything else is passthrough.

### What changes

- `vlinder timeline log` stops parsing git output and reformatting it — it runs `git log` directly
- `vlinder timeline fork <commit>` becomes `vlinder timeline checkout -b fork-<short> <commit>`
- Any future timeline feature is evaluated against: "can git already do this?"

### Commits advance the current branch

The GitDagWorker advances HEAD — whatever branch it points to. By default that's `main`. Sessions are distinguished by the `Session:` trailer in the commit message. Filtering by session is `git log --grep="Session: <id>"`. Filtering by entity is `git log --author=<name>`.

Forking a timeline is a git checkout:

```
cd ~/.vlinder/conversations
git checkout -b experiment <commit>
vlinder agent run todoapp      # new commits go to 'experiment'
```

The user now has two divergent timelines from the same point. `git diff main experiment` shows exactly how the agent behaved differently. Switching back to `main` and running again continues the original timeline. This is time travel — fork, explore, compare, return.

### Messages accumulate in the tree

Each commit's tree is a superset of the previous one. Every message gets its own directory. The tree grows — it never shrinks.

```
~/.vlinder/conversations/
├── 20260211-143052-cli-invoke/
│   ├── payload
│   └── diagnostics.toml
├── 20260211-143052-support-agent-request/
│   ├── payload
│   └── diagnostics.toml
├── 20260211-143053-infer.ollama-response/
│   ├── payload
│   ├── diagnostics.toml
│   └── stderr              (if non-empty)
├── 20260211-143054-support-agent-complete/
│   ├── payload
│   └── diagnostics.toml
├── agent.toml               (agent manifest)
├── platform.toml            (vlinder version, registry host)
└── models/                  (model manifests)
```

Directory names are `{YYYYMMDD-HHMMSS}-{sender}-{message_type}/`. The timestamp is the observed time — when the platform received the message. Natural `ls` sorting gives chronological order. The sender tells you who emitted this message.

Looking at the directory gives cumulative state — every message that was ever sent. The commit history gives the event log — `git log` shows when each message arrived, and `git diff HEAD~1 HEAD` shows exactly what the last message added.

### Working tree is always populated

The GitDagWorker uses git plumbing commands (`hash-object`, `mktree`, `commit-tree`, `update-ref`) for writes — no staging area, no merge conflicts. After each commit, it runs `git checkout -f` to sync the working tree to match the latest commit.

Users can `ls`, `cat`, or open files in an editor — no git knowledge required. Full history is still available via `git log` and `git show <commit>:path`.

### What doesn't change

- Route (ADR 063) stays as a domain model — it provides the `route` subcommand that groups messages into stops
- The git repo at `~/.vlinder/conversations/` stays — that's where timeline points

## Consequences

- `vlinder timeline` is a thin passthrough, not a custom viewer
- Users bring their own git knowledge — no new commands to learn
- Any git feature is automatically a timeline feature
- The `route` subcommand is the only domain-specific addition
- Custom parsing and formatting code in `timeline.rs` is removed
- The platform's value is in the commit format, not the viewer
- The conversations directory is browsable — `ls` shows all messages, `cat` reads any payload
- The directory is cumulative state, the commit log is the event log
- `git diff HEAD~1 HEAD` shows exactly what the last message added
- Forking a timeline is `git checkout -b` — true time travel with zero application code
