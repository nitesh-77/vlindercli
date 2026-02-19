# ADR 099: A Log is a List of Revertable States

## Status

Draft

## Context

`vlinder timeline log` shows the history of an agent run. But it's not
just a list of events — every entry is a state the system was in, and
every state is one the user can revert to.

A log entry isn't "something that happened." It's "somewhere you can go."

This distinction matters because it defines what a log entry must
contain: enough information for the user to decide whether to revert,
and a content-addressed identity (ADR 097) that makes the revert
possible.


## Decision

A log is a list of content-addressed states, each of which is a valid
target for `checkout`. Every entry in the log must:

1. Be identifiable — a content-addressed hash the user can pass to
   `checkout`
2. Be descriptive — enough context for the user to understand what
   state the system was in
3. Be revertable — the system can reconstruct that state from the
   identity alone


## Consequences

- Log entries are not just for reading — they're the input to time
  travel operations
- The quality of the log directly determines the usability of time
  travel: if the user can't understand a log entry, they can't decide
  whether to revert to it
- Every state that appears in the log must be a valid checkout target —
  no dangling references, no states that can't be restored
