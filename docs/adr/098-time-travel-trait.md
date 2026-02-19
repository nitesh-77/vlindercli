# ADR 098: Time Travel Trait

## Status

Draft

## Context

ADR 097 established that every protocol identity in Vlinder is a
content-addressed hash — a node in a Merkle DAG. This gives the
platform its core capability: time travel.

Today, time travel operations exist as concrete commands in
`vlinder timeline`:

| Command | Operation |
|---|---|
| `log` | See what happened — the full history |
| `route` | Follow the message chain for a session |
| `checkout` | Move to a specific point in history |
| `repair` | Branch from current position and re-execute |
| `promote` | Make the current branch main |

These are the user-facing time travel API. But they're implemented
directly against git — there's no trait that captures what "time travel"
means as a contract. The implementation is coupled to git as the
backing store.

Content-addressed identity (ADR 097) makes time travel possible. The
trait should capture what the user can do with it.


## Decision

The five timeline commands are trait methods:

| Method | Contract |
|---|---|
| `log` | Return the history of events from a point |
| `route` | Return the ordered message chain for a session |
| `checkout` | Move to a specific point in the DAG |
| `repair` | Fork from current position and re-execute |
| `promote` | Replace the main branch with the current one |

Git is one implementation. Any backing store that supports
content-addressed identity (ADR 097) could implement another.


## Consequences

- Time travel becomes a testable contract, not a git implementation
  detail
- Alternative backing stores (not git) could implement the same trait
- The platform's core differentiator ("AI agents that can time travel")
  is expressed in code, not just marketing
