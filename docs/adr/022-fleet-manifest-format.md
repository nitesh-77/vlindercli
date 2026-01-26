# ADR 022: Fleet Manifest Format

## Status

Accepted

## Context

Fleets compose multiple agents into a unit. We need a manifest format that:
- Identifies the fleet at runtime (daemon mode runs multiple fleets)
- Declares which agents belong to this fleet
- Designates which agent receives external requests
- Allows future extension (permissions, resource limits, inline agents)

## Decision

**Fleet manifest is `fleet.toml` at the project root.**

```toml
name = "finqa-fleet"
entry = "orchestrator"

[agents.orchestrator]
path = "agents/orchestrator"

[agents.worker]
path = "agents/worker"
```

### Fields

**`name`** (required) — Runtime identity for the fleet.
- Used in logs, management commands, API routing
- Must be unique when multiple fleets run in daemon mode

**`entry`** (required) — Name of the agent that receives external requests.
- References a key in `[agents.*]`
- The entry agent typically orchestrates other agents via `yield_to`

**`[agents.<name>]`** — Table of agents in this fleet.
- Key is the agent's name within the fleet (used by `entry`, `yield_to`)
- Each agent is defined by reference or inline

### Agent Definition

**By reference** — points to a directory with `agent.toml` (ADR 020):
```toml
[agents.orchestrator]
path = "agents/orchestrator"  # must contain agent.toml
```

**Inline** — embeds the agent manifest directly:
```toml
[agents.simple-parser]
code = "parsers/simple.wasm"
description = "Lightweight parser"

[agents.simple-parser.requirements]
models = []
services = []
```

Inline agents are useful for simple, fleet-specific components that don't warrant their own directory.

## Consequences

- Fleet identity is explicit (`name`), not derived from directory
- Agent names are first-class (table keys), paths are configuration
- `entry` references by name, avoiding path duplication
- Table format extensible for per-agent config (permissions, limits)
- Inline agents enable lightweight composition without directory overhead
- Referenced agents must follow ADR 020 (directory with `agent.toml`)
