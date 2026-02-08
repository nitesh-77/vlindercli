# ADR 058: Structured Logging

**Status:** Proposed

## Context

Vlinder's current logging setup is minimal: `tracing_subscriber::fmt()` outputs human-readable text to stderr with no timestamps, no file persistence, and no structured format. When things go wrong, the output vanishes when the process exits. There is nothing for the support fleet's log-analyst (ADR 057) to read.

The log-analyst mounts `~/.vlinder/` and has access to logs, timeline SHAs, config, SQLite databases — the full operational picture. But the logs directory is empty. The most capable agent in the support fleet has nothing to work with.

This isn't a general-purpose observability project. The logging system exists to serve one concrete consumer: the log-analyst agent. Every design choice follows from what makes that agent most effective.

## Decision

### Dual-output logging

Two `tracing` subscriber layers running simultaneously:

1. **stderr** — Human-readable format for the developer watching the terminal. Same as today but with timestamps added. Level-filtered via config/env (default: `info`).

2. **JSONL file** — Machine-readable structured logs written to `~/.vlinder/logs/vlinder.jsonl`. Always writes at `trace` level — everything gets captured. The log-analyst is an LLM; verbosity is a feature, not a problem.

### Log line schema

Every JSONL line is a self-contained JSON object:

```json
{
  "ts": "2025-01-15T14:30:22.451Z",
  "level": "info",
  "sha": "a1b2c3d4",
  "session": "s-9f8e7d",
  "agent": "log-analyst",
  "component": "runtime.container",
  "event": "dispatch.started",
  "msg": "Dispatching invocation to container",
  "data": { "port": 54321, "image": "localhost/vlinder-log-analyst:latest" }
}
```

Fields:

| Field | Source | Purpose |
|-------|--------|---------|
| `ts` | System clock | Temporal ordering, latency analysis |
| `level` | tracing level | Severity filtering |
| `sha` | `SubmissionId` | Trace correlation — the SHA from ADR 055's timeline |
| `session` | `SessionId` | Conversation grouping |
| `agent` | Span context | Which agent this relates to |
| `component` | Module path | Which system component emitted it |
| `event` | Structured field | Machine-parseable event type |
| `msg` | Log message | Human-readable description |
| `data` | Structured fields | Optional payload (ports, IDs, error details) |

### The SHA is the trace ID

The system timeline (ADR 055) assigns content-addressed SHAs to every interaction. The `SubmissionId` propagates through all messages. This is the trace ID — no separate tracing infrastructure needed.

The log-analyst correlates by SHA: given a user's error, find the SHA from the timeline, filter logs by that SHA, reconstruct the full execution across all agents and components. The SHA connects logs to the conversation store, to the registry, to agent storage — every data source the log-analyst can access under `~/.vlinder/`.

### Log verbosity

Logs are no-op from a business logic perspective. They must not harm the system — two hard constraints:

1. **The human terminal stays clean.** Verbose logging is invisible to the user. The stderr layer remains level-filtered (default `info`). The JSONL file captures everything silently. A user running `vlinder support` should never see a wall of trace output.

2. **Logs do not eat disk.** The file sink rotates automatically. When the current log file exceeds a size threshold, it is rotated and old files are pruned. The total log footprint is bounded. The default budget is generous enough for the log-analyst to have meaningful history, but never unbounded.

Within these constraints, the file sink captures at `trace` level because:

- The consumer is an LLM, not a human scrolling through pages
- Missing context directly limits the support agent's diagnostic ability
- We can always filter down; we cannot retroactively add detail we didn't capture

### Event taxonomy

Structured `event` fields for machine-parseable log analysis:

| Event | Component | When |
|-------|-----------|------|
| `message.received` | queue | Any message dequeued |
| `message.sent` | queue | Any message enqueued |
| `dispatch.started` | runtime | Container invocation begins |
| `dispatch.completed` | runtime | Container invocation ends |
| `dispatch.failed` | runtime | Container invocation errors |
| `inference.requested` | service_router | Agent calls /infer |
| `inference.completed` | service_router | Inference result returned |
| `delegation.sent` | service_router | Agent calls /delegate |
| `delegation.received` | runtime | DelegateMessage dequeued |
| `delegation.completed` | service_router | /wait returns result |
| `container.started` | runtime | Podman container launched |
| `container.stopped` | runtime | Podman container stopped |
| `session.started` | harness | New conversation session |
| `state.committed` | harness | Agent state written to timeline |
| `kv.get` | service_router | Agent reads from storage |
| `kv.put` | service_router | Agent writes to storage |

### File management

- Path: `~/.vlinder/logs/vlinder.jsonl`
- Rotation: `tracing-appender` rolling file appender, daily rotation, capped at 7 files. Total log footprint stays bounded without user intervention.
- The `logs_dir()` config helper (added in ADR 057) ensures the directory exists

## Scope

### Day One

- Add `tracing-appender` for file output with daily rotation (7 day retention)
- Add `tracing-subscriber` JSON layer writing to `~/.vlinder/logs/`
- Add timestamps to stderr output
- Propagate SHA and session through tracing spans
- Add structured event fields to existing `tracing::info!` calls in key paths

### Deferred

- Per-agent log files
- Log compaction (archive old JSONL to compressed format)
- Real-time log streaming for the log-analyst

## Consequences

- The log-analyst has material to work with — `vlinder support` becomes immediately useful
- Every execution is reconstructable from the log file by SHA
- The JSONL format is trivially parseable by the log-analyst agent (`json.loads()` per line)
- Daily rotation with 7 day retention keeps disk footprint bounded without user intervention
- Existing `tracing` instrumentation gains structured context without changing call sites (span propagation)
- The event taxonomy becomes a contract — the log-analyst's search logic depends on these event names
