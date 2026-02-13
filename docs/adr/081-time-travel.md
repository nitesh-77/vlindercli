# ADR 081: Time Travel

## Status

Draft

## Context

The platform captures every side effect as a content-addressed DAG node (ADR 067) and records state transitions in a SQLite state store (ADR 054, 055). Conversations are stored as git commits. The pieces exist, but the time travel experience has three problems:

1. **Tight coupling.** The harness reads state directly from the git repo on local disk. This collocates the CLI with the conversations repo and breaks in distributed deployments.

2. **Only completed turns.** You can only resume from a completed conversation turn (where the next natural step is user input). There is no way to resume from mid-execution — e.g., after a bad inference response.

3. **Git SHA as SubmissionId.** The SubmissionId is the git commit SHA. Cherry-picking a commit to a different branch produces a new SHA, breaking the link between the commit and its DAG nodes.

## Decision

### Git is a UX layer, the State Service is the source of truth

Git provides checkout, log, diff, branch, merge, cherry-pick — a UX for timeline manipulation that is too good to reimplement. But git is not the source of truth for state or identity. The State Service (ADR 079) owns both.

Only `vlinder timeline` touches git (via SSH for remote repos, local for local). All other components — harness, workers, runtime — read from and write to the State Service exclusively.

### Content-addressed SubmissionId

SubmissionId becomes a content-addressed hash of the user's input, stored in the State Store. Git commits reference the SubmissionId in a trailer, not the other way around.

This means cherry-picking a commit to another branch preserves the SubmissionId — same input, same hash, same DAG node linkage. The git commit SHA is just a pointer; the submission's identity is independent of which branch it lives on.

### `vlinder timeline checkout <node-hash>`

Checkout sets the cursor to a specific point in the DAG:

1. SSH to the git repo (local or remote)
2. `git checkout <sha>` — move HEAD to the corresponding commit
3. Read the `State:` trailer from that commit
4. Write the state to the State Service as the agent's current state

After checkout, `vlinder timeline log` shows only history up to that point. The user investigates, verifies they're at the right place.

### `vlinder timeline repair`

Repair is the one command git cannot provide — re-execute an agent from the current checkout point to completion:

1. Create a new branch from the current HEAD (`git checkout -b`)
2. Read state and session context from the State Service
3. Determine the next message type from the DAG node at HEAD
4. Re-execute: invoke the agent, send service requests, run to `CompleteMessage`
5. New DAG nodes and git commits land on the new branch

Repair runs non-interactively to completion. The user inspects the result afterward with `timeline log`.

### Cherry-pick for timeline construction

After repair, the user has a new branch with the corrected execution. Subsequent turns from the original branch can be selectively applied:

```
git cherry-pick <good-commit>
git cherry-pick <another-good-commit>
```

Cherry-pick creates new git SHAs, but the SubmissionId in the trailer is content-addressed — it survives the pick. DAG nodes remain queryable by SubmissionId regardless of which branch the commit lives on.

The user builds the corrected timeline deliberately, turn by turn. Turns that depended on the broken state are skipped — the butterfly effect is a conscious choice, not a hidden side effect.

### Promoting the repaired branch

After repair and cherry-picking, the repaired branch is the new reality. There is no merge step — the old main after the repair point is the broken timeline.

The user labels the broken timeline, then promotes the repaired branch:

```
git branch broken-2026-02-13 main
git branch -f main HEAD
git checkout main
```

`git branch broken-2026-02-13 main` saves the old main as a dated branch — a named bookmark to the timeline that went wrong. `git branch -f main HEAD` then moves the `main` label to wherever HEAD is — "this is main now."

The broken timeline is not deleted or hidden. It's a first-class branch the user can inspect, diff against, or cherry-pick from later. The reflog alone is not enough — an explicit label makes the broken timeline visible in `git branch` and `vlinder timeline log`.

## Consequences

- The harness is fully decoupled from the git repo — reads state from the State Service, never from git
- `vlinder timeline` is the single gateway for git operations (local or remote via SSH)
- SubmissionId is content-addressed (hash of user input), stored in the State Store, not derived from git SHAs
- Cherry-picks preserve submission identity — DAG tracing works across branches
- Mid-execution resume is possible — repair re-executes from any DAG node, not just completed turns
- Timeline construction is deliberate — the user cherry-picks valid turns, skips broken ones
- Git history is immutable — broken timelines are branches, not deleted history
