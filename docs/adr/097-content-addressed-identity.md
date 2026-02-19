# ADR 097: Content-Addressed Identity

## Status

Draft

## Context

The Vlinder platform records every side effect in a content-addressed
Merkle DAG. Several identities in the system already follow this model:

| Identity | Inputs | Where |
|---|---|---|
| SubmissionId | payload, session, parent submission | Conversation store |
| State commit | state entries | State store |
| DagNode | message content | DAG store |

These identities share three properties:

1. **Deterministic** — same inputs always produce the same hash
2. **Unique** — different inputs produce different hashes
3. **Verifiable** — you can recompute the hash to verify integrity

But this wasn't recognized as a unifying principle. It was implemented
ad-hoc, case by case. Other identities in the system use random UUIDs:

| Identity | Generation | Problem |
|---|---|---|
| Nonce (delegation reply) | UUID v4 | Not derivable from content |
| MessageId | UUID v4 | Not derivable from content |

Random UUIDs break the Merkle DAG model. You can't verify them. You can't
reconstruct them. Two identical operations produce different identities.
The DAG's guarantee — that content determines identity — has holes.

### What the routing key work revealed

ADR 096 introduced the Nonce type for delegation reply routing keys.
The initial design used UUID v4. But examining the question "what is a
nonce?" led to: a nonce is a resource, identified by a content-addressed
hash of its delegation dimensions. It's not random — it's derived from
the same content-addressed model as everything else.

This is when we recognized: the Merkle DAG isn't just an implementation
detail of the conversation store. It's the identity model for the entire
platform.


## Decision

### Every protocol identity is content-addressed

An identity in the Vlinder protocol is a hash of the content it
represents. The inputs to the hash are the dimensions that define that
identity within its bounded context.

| Identity | Inputs to hash |
|---|---|
| SubmissionId | payload, session, parent submission |
| State commit | state entries |
| DagNode | message content |
| Nonce | caller, target, submission, sequence |

If two operations have the same inputs, they produce the same identity.
If any input differs, the identity differs.

### Random generation is for ephemeral values only

UUIDs remain appropriate for values that are genuinely ephemeral — not
part of the protocol's observable state. SessionId (a conversation
session) is one example: it's created when a REPL starts and has no
content to derive from.

Protocol identities — values that appear in messages, routing keys, or
the conversation DAG — must be content-addressed.


## Consequences

- The Merkle DAG is the platform's identity model, not just a storage
  implementation detail
- Content-addressed identities enable time-travel debugging: replaying
  the same inputs produces the same DAG, making runs comparable
- Nonces become deterministic: same delegation at the same position
  in the conversation always produces the same reply routing key
- Random UUIDs in protocol identities are recognized as tech debt
- New protocol identities must specify their hash inputs as part of
  their design
