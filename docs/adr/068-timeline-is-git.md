# ADR 068: Timeline Is Git

## Status

Proposed

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
