# ADR 093: Branch-Scoped Message Subjects

## Status

Draft

## Context

Message subjects contain a SubmissionId that correlates request-reply pairs:

```
vlinder.{submission}.invoke.{harness}.{runtime}.{agent}
vlinder.{submission}.complete.{agent}.{harness}
```

SubmissionIds are content-addressed (ADR 081): `sha256(payload + session + parent)`. Same input at the same point in the conversation produces the same ID. That's the Merkle chain property — and it means two timelines forking from the same point with the same input produce identical subjects.

### Why this matters now

ADR 092 introduced `request()` and `invoke_and_wait()` on `MessageQueue`. The default implementation polls. NATS can do better: subscribe to the exact response subject, publish the request, await the response. Zero polling.

But this requires the subject to be globally unique. If two timelines share a submission ID, a core NATS subscription could receive a response from the wrong timeline.

### Timelines are branches

The conversation store is a git repo. `timeline repair` creates a branch and re-executes from a historical point. The new timeline shares the same session and can produce the same SubmissionIds.

Git branches are mutable named pointers — no immutable identity. Branch names change on promote/rename. We need an immutable timeline identifier that lives outside git.

## Decision

### 1. `timelines` table in the DAG store

The SQL DAG store gains a `timelines` table:

| column | type | meaning |
|---|---|---|
| id | integer primary key | timeline ID — goes into message subjects |
| branch_name | text | current git branch name (mutable) |
| parent_timeline_id | integer nullable | forked from which timeline |
| fork_point | text nullable | submission SHA at fork |
| created_at | timestamp | when the fork happened |
| broken_at | timestamp nullable | when this timeline was abandoned |

Row 1 is always `main`, created at first use. Each repair inserts a new row.

### 2. Timeline ID in message subjects

Every subject includes the timeline:

```
vlinder.{timeline}.{submission}.invoke.{harness}.{runtime}.{agent}
vlinder.{timeline}.{submission}.complete.{agent}.{harness}
vlinder.{timeline}.{submission}.req.{agent}.{service}.{backend}.{op}.{seq}
vlinder.{timeline}.{submission}.res.{service}.{backend}.{agent}.{op}.{seq}
vlinder.{timeline}.{submission}.delegate.{caller}.{target}
vlinder.{timeline}.{submission}.delegate-reply.{caller}.{target}.{uuid}
```

The `{timeline}.{submission}` pair is globally unique. Workers subscribe with a wildcard for timeline: `vlinder.*.*.invoke.*.*.{agent}` — they process work from any timeline.

### 3. Unique branch names

Repair branches use a monotonic counter: `repair-{date}-{n}`.

```
repair-2026-02-17-1
repair-2026-02-17-2
```

The counter scans existing branches matching `repair-{date}-*` and increments. The branch name is the human-facing label; the timeline ID is the protocol identifier.

### 4. Broken timelines are sealed

Once `broken_at` is set, no new messages can be sent on that timeline. The harness checks at session start and refuses to proceed.

Broken timelines are read-only archives — queryable in the DAG store, browsable in git, but not writable.

Forking from a broken timeline is allowed. The fork creates a new timeline row with `parent_timeline_id` pointing to the broken one. The broken timeline stays sealed.

### 5. Promote updates branch names, not timeline IDs

Promote does:

1. Old main's row: `broken_at = now()`, `branch_name = broken-{date}`
2. Repair row: `branch_name = main`

Timeline IDs never change. Subjects in the NATS stream still reference the original IDs. Only the human-facing branch name moves.

### 6. Lifecycle

```
start session     → harness reads current branch, looks up timeline ID
                    (creates row 1 for main on first use)

invoke            → timeline ID goes into InvokeMessage, propagates
                    to all subjects for this invocation

timeline repair   → DAG store inserts new timeline row
                    git creates branch repair-{date}-{n}
                    harness uses the new timeline ID

timeline promote  → old timeline: broken_at = now(), branch renamed
                    promoted timeline: branch_name = main
                    timeline IDs unchanged

timeline checkout → read-only navigation, no timeline mutation
```

## Consequences

- Subject uniqueness holds across timelines — enables core NATS subscriptions for request-reply
- Timeline identity lives in the DAG store, not in git — immutable, queryable
- The timelines table captures the full fork tree: who forked from whom, when, and why it broke
- Workers gain one extra wildcard in subscription patterns
- Broken timelines are permanently sealed — no accidental writes to abandoned timelines
- Timelines are a tree, not a line — you can fork from main, from repairs, from broken timelines
