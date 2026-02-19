# ADR 100: Storage Traits Are the Time Travel Contract

## Status

Draft

## Context

`vlinder timeline checkout` restores an agent's state from a previous
point in time. The system captures everything needed to reconstruct
that state:

| What | How it's captured |
|---|---|
| Agent files | ObjectStorage → StateCommit |
| Agent embeddings | VectorStorage → StateCommit |
| State snapshots | StateStore |
| Message flow | DagWorker (every observable message) |
| Inference calls + diagnostics | RequestMessage/ResponseMessage in DAG |
| Embedding calls + diagnostics | RequestMessage/ResponseMessage in DAG |
| Container diagnostics | CompleteMessage in DAG |

This is the full observable surface. Every side effect, every service
call, every diagnostic — all captured.

But none of this is represented in the domain as a unified concept.
ObjectStorage and VectorStorage are traits. DagWorker is a trait.
StateStore is a trait. The fact that they together form the time travel
contract is implicit — spread across implementations, not expressed as
a domain relationship.


## Decision

The storage traits collectively define the time travel contract. What
they capture is what gets hashed (ADR 097), what the log shows
(ADR 099), and what gets restored on checkout (ADR 098).

The observable surface is:

- **Agent state**: ObjectStorage (files) + VectorStorage (embeddings),
  snapshotted as content-addressed StateCommits
- **Conversation state**: DagWorker, recording every observable message
  including all diagnostics from inference, embedding, and container
  runtime

Everything the system records is restorable. Everything restorable
is observable. This relationship must be represented in the domain.


## Consequences

- The storage traits carry more weight than "where to put files" — they
  define the boundary of time travel
- State not saved through these traits is not restorable and does not
  appear in the DAG
- New storage capabilities must follow the same contract: save it,
  hash it, restore it
- The domain must express the relationship between storage traits and
  time travel — today it doesn't
