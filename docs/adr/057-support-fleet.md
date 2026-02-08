# ADR 057: Vlinder Support Fleet

**Status:** Accepted

## Context

Vlinder has fleet execution (ADR 056) but no fleet to run. The first fleet should validate the full stack: delegation, mounts, inference, while being genuinely useful. An interactive support agent that searches the user's logs and the project's documentation is better than static `--help` text.

## Decision

### Fleet composition

Three agents with a retrieval-grounded architecture: specialists retrieve data, the orchestrator reasons.

```toml
# fleets/support/fleet.toml
name = "vlinder-support"
entry = "support"

[agents.support]
path = "agents/support"

[agents.log-analyst]
path = "agents/log-analyst"

[agents.code-analyst]
path = "agents/code-analyst"
```

**`support`:** (entry agent) — Orchestrator. The only agent that calls the LLM. Routes queries (greeting / question / troubleshoot), delegates to specialists for retrieval, then answers grounded in the returned data.

**`log-analyst`:** Runtime state retrieval. Pure search, no LLM. Read-only mount to `~/.vlinder/` (logs, config, conversations, registry). Returns formatted log records, errors, config excerpts, and timeline data.

**`code-analyst`:** Documentation retrieval. Pure search, no LLM. Read-only mount to source tree. Always includes foundation docs (VISION.md, DOMAIN_MODEL.md, README.md). Filters stop words from queries. Returns formatted excerpts from ADRs and source code.

### Query routing

| Flow | LLM calls | Grounded? |
|------|-----------|-----------|
| Greeting ("hi") | 0 (keyword match) | N/A |
| Question ("what is a fleet?") | 2 (triage + answer) | Yes — doc excerpts |
| Troubleshoot ("agent crashes") | 2 (triage + synthesis) | Yes — logs + docs |

Every LLM call has retrieved context in its prompt. Fallbacks at each stage: no retrieval results returns a canned topic list, LLM failure returns raw excerpts.

### `vlinder support`

Syntactic sugar for `vlinder fleet run -p <bundled-support-fleet-path>`. Named `support` rather than `help` because clap reserves `help`.

### Why two specialists

Logs reveal what *happened* at runtime. Source + ADRs reveal what was *intended*. Neither alone gives the full picture. Separate agents also have separate mounts: the log-analyst can be reused in other fleets without source code access.

### Design principles

**Reference implementation.** The support fleet ships with Vlinder and demonstrates proper mount usage, delegation, and error handling.

**Log everything.** The log-analyst is an LLM, not a human. Err on the side of too much logging. Every message transition, inference call, and delegation should be logged with the submission SHA as trace ID (ADR 055).

**Gaps are the roadmap.** What the support agent can't answer reveals missing documentation and observability. The fleet is a forcing function for platform supportability.

## Scope

### Implemented

- Three agent containers with retrieval-grounded architecture
- Mount support in container runtime (`-v` flags from agent manifest)
- `vlinder support` subcommand

### Deferred

- Diataxis documentation — the content that makes the support agent genuinely useful
- Menu-driven triage (user self-selection instead of LLM classification)
- Embedding-based search (vector storage for smarter retrieval)
- Log streaming, auto-installation, result caching

## Consequences

- `vlinder support` works on first install — validates the full stack
- Support quality is bounded by available documentation — Diataxis docs are the next unlock
- Python container agents establish a pattern for non-Rust agent development
