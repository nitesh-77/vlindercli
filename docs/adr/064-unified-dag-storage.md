# ADR 064: Git as Agent Protocol

## Status

Proposed

## Context

Vlinder has two stores that capture chains of content-addressed nodes:

**ConversationStore** (ADR 054) — a git repo at `~/.vlinder/conversations/`. Each commit is a user message or agent response. Only sees user↔entry-agent turns.

**DagStore** (ADR 061) — SQLite with a `dag_nodes` table. Each node is an invoke/complete pair. Sees all agent boundaries including internal delegations.

They are structurally identical: content-addressed, append-only, parent-linked chains scoped by session. One uses git, the other uses SQLite. Both are DAGs.

ADR 063 identified the gap: fleet internals are invisible to the user. The DAG has the complete picture but lives in SQLite. The conversation store has git's tooling but only sees the outer conversation.

### The realization

Git is a content-addressed DAG store with refs for branching. That's not an analogy — that's literally what git was designed to be. The DagStore trait maps 1:1 onto git operations:

| DagStore operation | Git operation |
|---|---|
| `insert_node()` | `git commit` |
| `get_node(hash)` | `git show` |
| `get_session_nodes()` | `git log` |
| `get_children()` | `git log --children` |
| `ancestors()` | `git log --ancestry-path` |
| fork | `git checkout -b` |
| refs | `git branch` |

DAG nodes are one per agent invocation — the same frequency as conversation commits, which git already handles fine. The performance argument for SQLite (many small KV writes) doesn't apply here.

### What git gives us for free

If every agent interaction is a git commit, every git client becomes a Vlinder viewer:

- `git log --graph --all` — route visualization with branching
- `git diff <hash1> <hash2>` — payload comparison
- `git branch` — list all forks
- `git show <hash>` — inspect any node
- `git bisect` — binary search for where behavior diverged
- `git format-patch` / `git bundle` — export and share sessions
- `gitk`, VS Code, GitHub UI, `tig` — visualization without building anything
- `git push` — share a session
- `git verify-commit` — audit

This is an enormous UX surface we never have to build. Every developer already knows how to use it.

## Decision

**Git is the unified store for all agent interactions.** Every conversation — user↔agent and agent↔agent — is a git commit. The conversation doesn't use git for storage. The conversation *is* git.

The DagStore trait is the domain vocabulary for operations that git implements. The SQLite DagStore becomes the test double. Git is the production backend.

### What a fleet execution looks like

```
$ git log --graph --all --oneline

* f3a1b2c (HEAD -> main) support-agent → user: "Run podman pull..."
* d4e5f6a support-agent → code-analyst: "Check podman docs"
* b7c8d9e code-analyst → support-agent: "Requires pull before run"
* a1b2c3d support-agent → log-analyst: "Find errors"
* 9e8f7a6 log-analyst → support-agent: "dispatch.failed: container not found"
* 5c6d7e8 user → support-agent: "Why did my agent fail?"
```

That's not a debug view. That's the actual conversation. Every agent-to-agent delegation is a commit. The full route is visible in any git client.

### What changes

- The DagCaptureWorker writes git commits instead of SQLite rows
- All agent interactions (not just user↔entry-agent) become commits
- The SQLite `dag_nodes` table becomes `#[cfg(test)]` only
- `ConversationStore` and `DagStore` converge into one git-backed implementation
- Route (ADR 063) is `git log` with the right format

### What doesn't change

- DagNode as a domain type stays — it's the commit's content
- Content addressing stays — git does it natively
- Merkle chain stays — git's commit graph
- The DagStore trait stays — git implements it
- Fork stays `git checkout -b`

### Commit format

Each commit stores the agent interaction:

```
<from> → <to>

<payload_out summary>

Session: <session_id>
Agent: <agent_name>
Payload-In-Hash: <sha256>
Payload-Out-Hash: <sha256>
```

Full payloads stored as git blobs referenced from the commit tree. Summaries in the commit message for `git log` readability.

### Physical structure

The repo lives at `~/.vlinder/conversations/` — the same location as today's ConversationStore. What changes is what's inside it.

**Today**: the repo stores session JSON files. Each commit rewrites a JSON file that accumulates the conversation history. Git tracks changes to that file. The conversation is *in* the file.

```
~/.vlinder/conversations/
├── .git/
├── 2026-02-08T14-30-05Z_pensieve_abc12345.json
├── 2026-02-10T09-00-00Z_todoapp_def67890.json
```

**ADR 064**: there are no session files. Each commit IS one interaction. The commit tree holds payload blobs. The conversation is the commit graph itself.

```
~/.vlinder/conversations/
├── .git/
│   └── objects/   ← commits, trees, payload blobs
```

A commit's tree contains two blobs:

```
$ git ls-tree f3a1b2c
100644 blob abc123...  payload_in
100644 blob def456...  payload_out
```

The working tree may be empty. The data lives entirely in `.git/objects/`. The session is the chain of commits sharing the same `Session:` trailer — not a file on disk.

| | Today | ADR 064 |
|---|---|---|
| What's committed | Session JSON files | Payload blobs |
| Where the conversation lives | In the JSON file contents | In the commit graph |
| What a commit means | "I updated this session file" | "This agent interaction happened" |
| Fleet internals | Invisible | Every delegation is a commit |
| Branching | `git checkout -b` | Same |

This is the shift from document store to event log. Instead of updating a file that represents current state, each commit is an event that happened. Current state is derived from the log.

## Consequences

- Every git client is a Vlinder viewer — visualization, diffing, branching for free
- Fleet internals are visible in the same commit history as user conversations
- One store, one protocol, one data model
- `vlinder timeline route` becomes sugar over `git log`
- Sessions are portable — `git clone` to share, `git push` to back up
- The platform doesn't use git — the platform IS git
- SQLite DagStore remains as fast test double
- Developers debug agent conversations with tools they already know
