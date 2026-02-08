# ADR 054: Session Support

**Status:** Accepted

## Context

Every `invoke()` is independent. The harness creates a fresh `SubmissionId`, sends raw user input as the payload, and the agent has no knowledge of prior turns. Agents with persistent storage (pensieve) can work around this by storing and retrieving their own context, but conversational agents have no mechanism for multi-turn continuity.

Users expect follow-up questions to work: "summarize this article" → "what about the third point?" requires the agent to know what "this article" and "the third point" refer to.

## Decision

### SessionId

A `SessionId` groups multiple submissions into a conversation. Created by the CLI when the REPL starts.

Format: `ses-{uuid}`.

`SessionId` is a required field on `InvokeMessage` — it flows through to the runtime and is passed to the agent via the HTTP bridge (as a header). Agents that care about session identity can scope storage by conversation. Agents that don't can ignore it. Every invoke belongs to a session, even one-shot calls (a session with one turn). Git is a hard dependency and the conversation store always exists, so sessions are free.

### Storage: Git-backed JSON

Sessions are stored as JSON files in `~/.vlinder/conversations/`, which is a git repository.

- One file per session: `{timestamp}_{agent}_{session_id_short}.json`
- Git commit after each message (user input and agent response are separate commits)
- Repo initialized on first session creation (if directory exists but isn't a git repo, init it)

The git repo is the user's data. They can read, branch, diff, and inspect conversations with standard tooling. No proprietary format, no export needed.

Session file schema:

```json
{
  "open": null,
  "session": "ses-...",
  "agent": "pensieve",
  "history": [
    {"user": "summarize this article", "submission": "a1b2c3d", "at": "2026-02-08T14:30:05Z"},
    {"agent": "This article discusses...", "at": "2026-02-08T14:30:47Z"},
    {"user": "what about point 3?", "submission": "b3c4d5e", "at": "2026-02-08T14:31:02Z"},
    {"agent": "Point 3 discusses...", "at": "2026-02-08T14:31:45Z"}
  ]
}
```

`open` is the first field — the first thing you see when you open the file. It holds the current unanswered question as a string, or `null` when the conversation is at rest. History entries carry richer metadata (`submission`, `at`) for tracing and auditability.

When a question is pending:

```json
{
  "open": "what about point 3?",
  "session": "ses-...",
  "agent": "pensieve",
  "history": [
    {"user": "summarize this article", "submission": "a1b2c3d", "at": "2026-02-08T14:30:05Z"},
    {"agent": "This article discusses...", "at": "2026-02-08T14:30:47Z"}
  ]
}
```

### Git commit messages

Each message is a separate commit. Subject is the role, body is the full content, trailers carry metadata.

User input commit (its SHA becomes the SubmissionId — no self-referential trailer):

```
user

summarize this article

Session: ses-abc123
```

Agent response commit (references the SubmissionId = parent commit's SHA):

```
agent

This article discusses several topics including the history of
computing, advances in programming languages, and reflections
on 25 years of personal experience.

Session: ses-abc123
Submission: a1b2c3d
```

This makes `git log` read like a conversation:

```
git log --reverse --format="%n> %s%n%b" -- 2026-02-08_pensieve_abc123.json
```

```
> user
summarize this article

> agent
This article discusses several topics...

> user
what about point 3?

> agent
Point 3 discusses...
```

`git log --oneline --reverse` gives the overview:

```
a1b2c3d user
f2e3608 agent
b3c4d5e user
c4d5e6f agent
```

### Invoke payload: history as string

The harness builds a text payload that includes conversation history followed by the current input. The agent receives a single string — it doesn't know about sessions, JSON, or git.

Payload format:

```
User: summarize this article
Agent: This article discusses...
User: what about the third point?
```

Simple `Role: content` lines. Agents that don't care about history just see extra context. Agents that do get continuity for free.

### SubmissionId = commit SHA

The user input commit happens before `invoke()`. The commit SHA becomes the SubmissionId:

1. Harness writes JSON + `git commit` → SHA = `a1b2c3d`
2. `SubmissionId` = `a1b2c3d`
3. All queue messages carry `a1b2c3d` — every service call (kv, embed, infer) is tagged with a git commit

No prefix. The Rust newtype provides type safety. NATS subject position provides context. The raw SHA is directly pasteable into `git show`.

This makes the git commit chain a **logical clock** for the entire system:
- Every commit knows its parent → causal ordering for free
- `git log` IS the event log
- Branch a conversation → the causal graph forks
- SubmissionId in NATS subjects → `git show <sha>` → exact conversation state

Storage engines receive the SHA via `SubmissionId` on every `RequestMessage`. They can use it for versioning, snapshots, or replay — each engine handles this internally. The git merkle DAG is the external coordination layer; storage internals are independent.

### Lifecycle

1. REPL starts → harness creates `SessionId`, creates JSON file with `"open": null` and empty `"history"`
2. User types input → harness sets `open` to the user's question, **git commit** → returns SHA
3. `SubmissionId` = SHA
4. Harness reads `history`, builds payload with history + current input
5. `invoke()` sends enriched payload to agent with SHA-derived SubmissionId
6. Agent responds → harness sets `open` to `null`, appends completed pair to `history`, **git commit** (references SubmissionId in trailer)
7. Repeat from step 2

Two commits per turn — one for user input, one for agent response:

- **Crash resilience**: if the process dies mid-inference, the user's input is already committed
- **Granular branching**: fork after a question to try a different model, or fork after a response to try a different follow-up
- **Honest timeline**: `git log` shows exactly who said what, when
- **Logical clock**: commit parent chain = causal ordering without custom infrastructure

### Concurrency

Not a concern for the CLI harness — single user, sequential inputs. Git index lock contention is theoretically possible with multiple REPL sessions but unrealistic in practice. No retry logic needed on day one.

Future harnesses solve this differently:
- **Web harness**: could be backed by GitHub/GitLab, which handle concurrent commits natively
- **Distributed mode**: each machine has its own `~/.vlinder/conversations/`. Users can `git push` to a remote for cross-machine history — sync via cron or hooks

## Consequences

- Multi-turn conversations work without agents opting in to any session protocol — history arrives in the payload
- Agents that want session awareness (e.g., scoping storage by conversation) can read the `SessionId` via SDK
- Conversation data is human-readable (JSON) and inspectable via git
- `git log` per session file reads like a chat transcript
- Git branching enables conversation forking without custom UX
- The harness owns session lifecycle — agents receive sessions, they don't manage them
- SubmissionId = commit SHA makes every queue message traceable to a git commit
- The commit parent chain is a logical clock — causal ordering without custom infrastructure
- Storage engines receive the SHA on every operation and can independently implement versioning/snapshots (future ADR)
- Git is a hard dependency (like NATS, SQLite, Ollama)
- Payload size grows with conversation length — truncation/summarization strategy deferred
- File server with HTML templates can render sessions for browser viewing (future ADR)
- Session resume (`vlinder run --session <id>`) is naturally supported by the storage design (future work)
