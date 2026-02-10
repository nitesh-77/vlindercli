# ADR 066: ~/.vlinder as Platform Root

## Status

Proposed

## Context

Vlinder's components have accumulated in `~/.vlinder/` organically:

- `~/.vlinder/agents/` — agent manifests and data (registry)
- `~/.vlinder/models/` — model path references (Ollama)
- `~/.vlinder/conversations/` — git repo (conversation store, ADR 054)
- `~/.vlinder/state/` — SQLite (state store, ADR 055)
- `~/.vlinder/dag/` — SQLite (DAG store, ADR 061)

The daemon, Podman, Ollama, and NATS all run locally. Everything the platform needs — processes, configuration, and data — is under one directory.

This wasn't designed. It emerged. But it's worth naming, because it's a property we want to preserve.

### Control plane and data plane in one place

**Control plane** — what runs. Agent definitions, model references, the daemon process, container runtime (Podman), inference (Ollama), message bus (NATS).

**Data plane** — what happened. Conversations (git projection), state snapshots (SQLite), agent interaction DAG (SQLite analytical projection). All are materialized views of the NATS event stream (ADR 065).

Both live under `~/.vlinder/`. The entire platform — configuration, runtime, and history — is a directory.

### What this gives us

**Portable.** `cp -r ~/.vlinder ~/backup` backs up the entire platform. Move it to another machine with Podman and Ollama installed, and everything works — agents, conversations, state, history.

**Deletable.** `rm -rf ~/.vlinder` removes all traces. No orphaned databases, no stale cloud resources, no dangling configuration in three different locations.

**Inspectable.** `ls ~/.vlinder/` shows what the platform is. `cd ~/.vlinder/conversations && git log` shows what it did. No hidden state. No opaque data directories. Standard tools (git, sqlite3, ls) are sufficient to understand everything.

**Shareable.** The git projection means `git push` from `~/.vlinder/conversations/` backs up the data plane to anywhere git can reach. `git clone` on another machine recreates the conversation history.

## Decision

**`~/.vlinder/` is the platform root.** All platform state — control plane and data plane — lives under this single directory. This is a constraint, not a default.

```
~/.vlinder/
├── agents/              ← agent manifests (control plane)
├── models/              ← model references (control plane)
├── conversations/       ← git repo (data plane, git projection)
├── state/               ← SQLite (data plane, state store)
└── dag/                 ← SQLite (data plane, analytical projection)
```

New components go under `~/.vlinder/`. If a component's data can't live there, that's a design smell worth investigating.

## Consequences

- The platform is a directory — portable, deletable, inspectable
- `cp -r` is a valid backup strategy; `rm -rf` is a valid uninstall
- Standard tools (git, sqlite3, ls) can inspect every part of the platform
- No hidden state in system directories, databases, or cloud services
- The distributed mode (Supervisor + NATS + gRPC) is opt-in scaling, not a requirement — the local directory is always the starting point
- New components have a clear home and a clear constraint
