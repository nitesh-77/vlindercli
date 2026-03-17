# ADR 117: Session-scoped routing keys

## Status
Draft

## Context
The NATS subject scheme was designed for single-user, single-machine
orchestration. The original runtime used WASM agents on a single process.
Routing keys use `(branch, submission)` as the primary dimensions — session
is metadata in message headers, not a structural routing dimension.

### The fork bug that exposed this
`SubmissionId::content_addressed` hashes `(payload, session_id, parent_submission)`.
Branch is not in the hash. When a fork creates a new branch and the user sends
the same input that was sent on the original branch, the fork's invoke gets the
**same submission ID** as the original.

`receive_response` matches on `vlinder.*.{submission}.res.{service}.{backend}.*.*.*`
— it wildcards the branch. The JetStream consumer (DeliverPolicy::All) replays
from the beginning of the stream and picks up the **stale response from the
original branch** instead of waiting for the fresh one.

The agent receives empty KV data (from the original invoke when no todos existed)
rather than the correct data at the fork point.

### Broader design gap
The bug is a symptom of a deeper problem: the routing scheme doesn't model
multi-tenancy. Session is the tenancy boundary (ADR 116 discussion), but
it's invisible to NATS subject routing. This means:

1. **Cross-branch collision** — same content on different branches produces
   identical submission IDs, causing stale response pickup (the fork bug).
2. **Sidecar is per-agent, not per-session** — one sidecar handles all sessions
   sequentially. Two concurrent users queue behind each other.
3. **Worker subscriptions are global** — the KV worker subscribes to all
   requests for `(service=kv, operation=get)` across all sessions.
4. **Consumer naming is filter-derived** — overlapping filters share JetStream
   consumers, causing round-robin delivery instead of isolation.

## Decision

### 1. Session and branch become first-class routing dimensions
Current subject scheme:
```
vlinder.{branch}.{submission}.{type}.{...}
```

New subject scheme:
```
vlinder.{session}.{branch}.{submission}.{type}.{...}
```

Session is the root isolation boundary. Branch scopes execution within a
session. Submission identifies a specific turn on a specific branch.

This hierarchy matches the domain model:
- Session owns branches
- Branch owns the DAG chain
- Submission identifies a turn on a branch

### 2. Branch ID included in submission hash
`SubmissionId::content_addressed` adds `branch_id` to the hash:
```
SHA-256(payload || \0 || session_id || \0 || branch_id || \0 || parent_submission)
```

Same content on different branches produces different submission IDs.
The content-addressed property is preserved within a branch — same input
at the same conversation position still produces the same hash.

### 3. Receive filters use exact dimensions, not wildcards
`receive_response` currently wildcards the branch:
```
vlinder.*.{submission}.res.{service}.{backend}.*.*.*
```

After this change, both session and branch are pinned:
```
vlinder.{session}.{branch}.{submission}.res.{service}.{backend}.*.*.*
```

No cross-branch or cross-session message pickup is possible.

### 4. RoutingKey variants include session
Every `RoutingKey` variant adds `session: SessionId` alongside the existing
`branch: BranchId` (renamed from `timeline`).

## Consequences
- Fork bug is fixed: different branches produce different submission IDs and
  different NATS subjects.
- Multi-session isolation at the routing level — NATS subjects naturally
  partition by session.
- Per-session sidecars become possible in the future (subscribe to
  `vlinder.{session}.>`).
- Workers can choose their isolation granularity: session-specific subscription
  vs wildcard across sessions.
- The `timeline` field in RoutingKey and message types should be renamed to
  `branch` as a follow-up (separate ADR or mechanical rename).
- All NATS consumers will need to be recreated (filter patterns change).
  In practice this happens automatically — existing consumers expire via
  `inactive_threshold`.
