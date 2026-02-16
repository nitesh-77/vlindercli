# ADR 088: Timeline UX

## Status

Draft

## Context

ADR 081 established time travel mechanics: content-addressed SubmissionId, checkout, repair, cherry-pick, branch promotion. ADR 068 decided conversations live in git. The infrastructure exists — but the UX is raw git.

The `DagStore` trait abstracts the storage boundary. Timeline commands target the trait, not git directly.

### Conversation store scaling gradient

The conversation store can evolve through a gradient of implementations as scale demands grow:

| Level | Backend | Graduation signal |
|---|---|---|
| 1 | Git (shell) | Commit latency > 5ms or > 50K objects/day |
| 2 | libgit2 | Average blob > 1KB, repo > 1GB/week |
| 3 | libgit2 + LFS | Multiple concurrent writers, need HA |
| 4 | Gitaly | Git model itself becomes the bottleneck |
| 5 | FoundationDB | — |

Each step standardises access patterns further. The timeline commands don't change.

## Decision

Deferred.
