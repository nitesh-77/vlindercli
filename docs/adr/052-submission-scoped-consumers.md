# ADR 052: Submission-Scoped Consumers

**Status:** Accepted

## Context

Two bugs stem from how `receive_complete` creates NATS consumers:

1. **Consumer garbage collection.** `receive_complete("cli")` creates a single NATS consumer shared across all invocations. Between REPL inputs, the consumer idles and NATS garbage-collects it. The second invocation's completion is never received.

2. **Multi-CLI interference.** Two CLI instances share the same consumer. In WorkQueue mode, one steals the other's completion messages.

Both stem from the same root cause: consumers are scoped to the harness type (`cli`), not to an invocation.

## Decision

`receive_complete` takes a `SubmissionId` and creates a consumer filtered to `vlinder.{submission}.complete.*.{harness}`. Each invocation gets its own consumer. NATS garbage-collects it after the invocation completes — that is the desired behavior.

## Flow

1. User types input — harness creates a new `SubmissionId`
2. Harness invokes agent, passing the submission ID through
3. Submission-scoped consumer created for completion polling
4. Agent processes, responds via the submission's complete subject
5. Harness receives completion on the scoped consumer
6. Consumer abandoned, NATS GCs it
7. Repeat from step 1

## Consequences

- Fixes consumer GC: each consumer lives exactly as long as its invocation
- Fixes multi-CLI interference: each CLI instance polls its own submission-scoped consumer
- No shared mutable state between invocations or CLI instances
- Conversation history and session management are separate concerns (future ADR)
