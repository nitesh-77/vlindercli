# Observability

How to read, search, and debug with Vlinder's structured logs.

---

## Where Logs Live

```
~/.vlinder/logs/
  vlinder.2025-01-15.jsonl
  vlinder.2025-01-16.jsonl
  vlinder.2025-01-17.jsonl
  ...
```

Daily rotation, 7 files max. Old files are pruned automatically.
The directory is created on first run. Total disk footprint is bounded
without user intervention.

---

## Two Outputs, One `tracing` Call

Every `tracing::info!(...)` in the codebase writes to two places simultaneously:

| Layer | Format | Level | Target |
|-------|--------|-------|--------|
| **stderr** | Human-readable, with timestamps | Configurable (default `info`) | Your terminal |
| **JSONL file** | Machine-readable JSON, one object per line | Always `trace` | `~/.vlinder/logs/` |

The file layer captures everything. The terminal stays clean. A single
invocation produces a complete trace in the log file without any noise
on screen.

---

## Log Line Anatomy

Each JSONL line is a self-contained JSON object:

```json
{
  "timestamp": "2025-01-15T14:30:22.451123Z",
  "level": "INFO",
  "fields": {
    "message": "Dispatching to container",
    "event": "dispatch.started"
  },
  "target": "vlindercli::runtime::container",
  "spans": [
    {
      "name": "invoke",
      "sha": "a1b2c3d4e5f6...",
      "session": "ses-9f8e7d6c",
      "agent": "echo-agent"
    }
  ]
}
```

| Field | What It Tells You |
|-------|-------------------|
| `timestamp` | When it happened |
| `level` | Severity: `TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR` |
| `fields.message` | Human-readable description |
| `fields.event` | Machine-parseable event type (see taxonomy below) |
| `target` | Rust module path — which component emitted it |
| `spans` | Active span context — carries `sha`, `session`, `agent` |

### The SHA Is the Trace ID

The `sha` field in spans is the `SubmissionId` — the git SHA from the
conversation timeline. It propagates through every message in the system.
To reconstruct a full execution, filter logs by SHA.

---

## Controlling Log Levels

### Terminal (stderr)

**Config file** (`~/.vlinder/config.toml`):

```toml
[logging]
level = "info"           # trace, debug, info, warn, error
llama_level = "error"    # suppress llama.cpp noise
```

**Environment variable** (overrides config):

```bash
VLINDER_LOGGING_LEVEL=debug vlinder agent run -p agents/echo-agent/
```

**`RUST_LOG` env filter** (overrides everything, full tracing-subscriber syntax):

```bash
RUST_LOG=vlindercli::runtime=trace vlinder agent run -p agents/echo-agent/
```

### File (JSONL)

Always captures at `trace` level. This is not configurable by design —
the log-analyst agent needs maximum context, and you can always filter
after the fact.

---

## Searching Logs

### With `jq`

```bash
# All events for a specific SHA
cat ~/.vlinder/logs/*.jsonl | jq 'select(.spans[]?.sha == "a1b2c3d4")'

# All errors
cat ~/.vlinder/logs/*.jsonl | jq 'select(.level == "ERROR")'

# All dispatch events
cat ~/.vlinder/logs/*.jsonl | jq 'select(.fields.event | startswith("dispatch"))'

# Events for a specific agent
cat ~/.vlinder/logs/*.jsonl | jq 'select(.spans[]?.agent == "echo-agent")'

# Timeline of events in a single invocation
cat ~/.vlinder/logs/*.jsonl \
  | jq -r 'select(.spans[]?.sha == "a1b2c3d4") | "\(.timestamp) \(.fields.event // .fields.message)"' \
  | sort
```

### With `grep`

JSONL is grep-friendly — one record per line:

```bash
# Quick search for errors
grep '"ERROR"' ~/.vlinder/logs/*.jsonl

# Find all delegation events
grep '"delegation\.' ~/.vlinder/logs/*.jsonl

# Find events for a SHA prefix
grep 'a1b2c3d4' ~/.vlinder/logs/*.jsonl
```

### With the Log-Analyst Agent

```bash
vlinder support
```

The support fleet's log-analyst reads these same JSONL files and
correlates them with config and conversation timeline. It supports
structured queries:

```
> Why did the last invocation fail?
> What happened for SHA a1b2c3d4?
> Show me all delegation events for echo-agent
```

---

## Event Taxonomy

Structured `event` fields in `fields.event`:

### Invocation Lifecycle

| Event | Component | When |
|-------|-----------|------|
| `invoke.started` | harness | User submits input, job created |
| `dispatch.started` | container runtime | Container invocation begins |
| `dispatch.completed` | container runtime | Container invocation ends |
| `dispatch.failed` | container runtime | Container start or dispatch error |

### Container Lifecycle

| Event | Component | When |
|-------|-----------|------|
| `container.started` | container runtime | Podman container launched |
| `container.stopped` | container runtime | Podman container stopped |

### Service Calls

| Event | Component | When |
|-------|-----------|------|
| `service.request` | service router | Agent SDK call dispatched to queue |
| `service.polling` | service router | Waiting for service response |
| `service.response` | service router | Service response received |

### Delegation (Agent-to-Agent)

| Event | Component | When |
|-------|-----------|------|
| `delegation.sent` | service router | Agent delegates to another agent |
| `delegation.received` | container runtime | DelegateMessage dequeued for target |
| `delegation.completed` | service router | Delegation result returned to caller |

---

## Correlating Across Data Sources

The `sha` connects logs to everything else under `~/.vlinder/`:

```
SHA "a1b2c3d4"
    │
    ├── Logs:          grep a1b2c3d4 ~/.vlinder/logs/*.jsonl
    │                  (full execution trace)
    │
    ├── Timeline:      cd ~/.vlinder/conversations/<agent> && git show a1b2c3d4
    │                  (what the user said, what the agent replied)
    │
    ├── State:         git log --format='%s%n%b' | grep -A1 'Submission: a1b2c3d4'
    │                  (State: trailer → state commit hash in agent's state.db)
    │
    └── Config:        cat ~/.vlinder/config.toml
                       (system configuration at the time)
```

---

## Debugging Recipes

### "My agent invocation hung"

```bash
# Find the SHA from the most recent invocation
cat ~/.vlinder/logs/*.jsonl | jq -r 'select(.fields.event == "invoke.started") | .spans[]?.sha' | tail -1

# Then trace it
SHA=<paste>
cat ~/.vlinder/logs/*.jsonl | jq "select(.spans[]?.sha == \"$SHA\")" | jq -r '"\(.timestamp) \(.level) \(.fields.event // "-") \(.fields.message)"' | sort
```

Look for `dispatch.started` without a matching `dispatch.completed` —
that tells you the container is stuck. Check for `service.request` without
`service.response` — that tells you a worker isn't responding.

### "Container won't start"

```bash
cat ~/.vlinder/logs/*.jsonl | jq 'select(.fields.event == "dispatch.failed")'
```

The `error` field in the log record will contain the podman error message.

### "Delegation isn't working"

```bash
# See the full delegation flow
cat ~/.vlinder/logs/*.jsonl | jq 'select(.fields.event | startswith("delegation"))'
```

You should see `delegation.sent` → `delegation.received` → `dispatch.started` →
`dispatch.completed` → `delegation.completed`. A missing step tells you where
it broke.

---

## Related Documentation

- [ADR 058](adr/058-structured-logging.md): Design decision for structured logging
- [ADR 057](adr/057-support-fleet.md): Support fleet and log-analyst agent
- [Timeline](TIMELINE.md): How SHAs connect to the conversation store
- [Request Flow](REQUEST_FLOW.md): How requests travel through the system
