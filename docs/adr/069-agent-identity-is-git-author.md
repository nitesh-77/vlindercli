# ADR 069: Agent Identity Is Git Author

## Status

Proposed

## Context

Every git commit has an author. Every agent has an identity (its registered name in the registry). In the git projection (ADR 064), every agent interaction is a commit.

The agent that produced the message IS the commit's author. This isn't a mapping — it's a direct use of git's existing identity model.

### Provenance

The DAG worker writes commits, not the agents. The worker observes NATS traffic and knows which agent produced each message from the subject. The author field is set by the **platform**, not by the agent. It's a verified claim, not a self-declaration.

If the repo is locked down so only the DAG worker can commit, and the worker signs its commits, then `git verify-commit` proves: this message was produced by this agent, observed by the platform, and no one has tampered with it since. That's cryptographic provenance over the full conversation history.

No agent could have faked another agent's commit. No one could have inserted a message after the fact. The audit trail is the git graph, and git's existing signing infrastructure makes it verifiable.

## Decision

**The agent's identity is the git commit author.** When a DAG worker writes a commit for a message, the `--author` field is the agent (or harness, or service) that sent it.

```
Author: support-agent <support-agent@vlinder>

invoke: cli → container.support-agent

Session: ses-abc123
Submission: sub-1
```

For user-originated messages (invoke from harness), the author is the harness type:

```
Author: cli <cli@vlinder>

invoke: cli → container.support-agent
```

For service responses (inference, storage), the author is the service:

```
Author: infer.ollama <infer.ollama@vlinder>

response: infer.ollama → support-agent
```

### What git gives us for free

| Git command | What it shows |
|---|---|
| `git log --author=support-agent` | Everything this agent did |
| `git log --author=infer.ollama` | Every inference call and response |
| `git shortlog -s` | Commit count per agent — activity summary |
| `git shortlog -sn` | Agents ranked by activity |
| `git blame` | Which agent (or service) produced each message |
| `git log --author=cli` | Every user interaction |
| `git verify-commit <hash>` | Cryptographic proof of provenance |

### Author format

```
<agent-name> <agent-name@vlinder>
```

The email portion uses `@vlinder` as the domain — simple, recognizable, no real email needed. Git requires the email field; this satisfies it without pretending agents have email addresses.

### Signing

The DAG worker holds the signing key. Commits are signed at write time. `git verify-commit` validates the chain. The platform guarantees: this author produced this message, and the commit hasn't been altered.

This is opt-in — signing adds cost. But the infrastructure is git's, not ours.

## Consequences

- Agent identity maps directly to git's author model — no custom metadata needed
- The platform sets the author, not the agent — it's a verified claim
- `git log --author` is agent filtering for free
- `git shortlog` is agent activity reporting for free
- `git verify-commit` is cryptographic audit for free
- Every git visualization tool that shows authors now shows agent identity
- No one can fake a commit from another agent if the repo is locked down
- No new identity or signing system to build — git already has both
