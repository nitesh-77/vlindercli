# ADR 057: Vlinder Support Fleet

**Status:** Proposed

## Context

Vlinder has fleet execution (ADR 056) but no fleet to run. The first fleet should validate the entire stack — delegation, fleet context injection, agent mounts, inference — while being genuinely useful rather than a throwaway demo.

CLI tools ship static `--help` text. Vlinder can do better: an interactive support agent that has access to the user's logs and the project's source code and documentation. Instead of searching docs manually, users describe their problem and get a classified response with actionable next steps.

This is dogfooding. The platform's first fleet is a support tool for the platform itself.

## Decision

### The support fleet

Three agents, each with distinct capabilities:

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

**`support`** (entry agent) — The triage orchestrator. Receives the user's question, delegates to both specialists in parallel, waits for both reports, then classifies the situation and formats a response. Uses inference for classification and synthesis. No file access — pure orchestration.

**`log-analyst`** — The runtime state specialist. Has a read-only mount to `~/.vlinder/`, which gives it access to the full operational picture:

- `logs/` — what happened at runtime (errors, message transitions, inference calls)
- `conversations/` — the system timeline with content-addressed SHAs linking every interaction
- `config.toml` — how the system is configured (queue backend, model endpoints, worker counts)
- `registry.db` — what's registered (agents, models, runtimes)
- `agents/*/` — per-agent SQLite databases (object storage, vector storage state)

Given a user's error description, the agent searches logs, correlates SHAs from the timeline, checks configuration for mismatches, and queries databases for registered state. Uses inference to synthesize across all sources and extract root causes. The name "log-analyst" understates what it does — it's a runtime state analyst — but the name is kept because "analyze what happened" is how users think about it.

**`code-analyst`** — The design intent specialist. Has a read-only mount to the source tree and ADR directory. Given a user's question, finds relevant code paths and documentation. Uses inference to explain whether behavior is by-design, a known limitation, or a gap.

### Five response categories

The entry agent classifies every interaction into exactly one category:

1. **Misconfiguration** — The user's setup is wrong. Response includes the correct configuration with the specific setting to change.

2. **Bug** — The observed behavior contradicts the design intent. Response includes a formatted bug report: title, description, reproduction steps.

3. **Feature request** — The user wants something the platform doesn't support yet. Response includes a formatted feature request: title, description, rationale.

4. **Author guide** — The user wants to build something new on the platform. Response includes the relevant manifest format, container contract, SDK endpoints, and a scaffold tailored to their use case.

5. **Out of scope** — The issue is unrelated to Vlinder (e.g., NATS server configuration, Docker networking, OS-level problems). Response acknowledges this and points the user elsewhere.

The classification is not a rigid routing table — the LLM synthesizes both specialists' reports and picks the category that best fits. A single question might start as "misconfiguration" but the log analyst reveals it's actually a bug.

### `vlinder support` as syntactic sugar

```
vlinder support
```

This is equivalent to `vlinder fleet run -p <bundled-support-fleet-path>`. The support fleet is installed as part of the standard Vlinder installation. Users don't need to know about fleets to get interactive support. Named `support` rather than `help` because clap reserves `help` as a built-in subcommand.

The command starts the support fleet's REPL. The user asks questions in natural language. The orchestrator delegates, collects evidence, classifies, and responds.

### Why two specialists instead of one

The log analyst and code analyst have fundamentally different knowledge:

- **Logs** reveal what *actually happened* at runtime — error sequences, timing, state transitions. The log analyst can correlate a 503 error with a consumer creation 30 seconds prior and identify a staleness issue.

- **Source + ADRs** reveal what was *intended* — the design rationale, the expected behavior, the known limitations. The code analyst can find the relevant ADR and explain whether the behavior is by-design.

Neither alone gives the full picture. A misconfiguration looks like a bug if you only read logs. A bug looks like a misconfiguration if you only read source code. The orchestrator needs both perspectives to classify correctly.

The specialists also have different mount requirements. Combining them into one agent would mean one container with access to both logs and source — possible, but it loses the composability benefit. The log analyst can be reused in a "monitoring fleet" without dragging in source code access.

### Mount support in container runtime

Container agents currently receive only the bridge URL environment variable. For the support fleet, agents need read-only access to host directories.

The container runtime maps agent mounts as podman volume flags:

```
podman run -d \
  -p :8080 \
  -e VLINDER_BRIDGE_URL=... \
  -v /Users/x/.vlinder:/vlinder:ro \
  localhost/log-analyst:latest
```

The mount paths are declared in the agent manifest (ADR 020). The container runtime reads them and adds `-v` flags. Read-only mounts use `:ro`. This is a small change in `ensure_container()`.

### Agent implementation

Each agent is a container image with a minimal HTTP server:

- `GET /health` — returns 200
- `POST /invoke` — processes the request

The agents use the bridge URL for platform services:
- `POST $VLINDER_BRIDGE_URL/infer` — LLM inference
- `POST $VLINDER_BRIDGE_URL/delegate` — invoke another agent
- `POST $VLINDER_BRIDGE_URL/wait` — collect delegation result
- `POST $VLINDER_BRIDGE_URL/kv/get` — read from storage

Python (Flask) for the proof of concept. Small images, fast iteration, widely understood. Production agents can be rewritten in any language.

### Design principles

**The support fleet is the reference implementation.** It ships on every system that has Vlinder. It should be an example of what good looks like — proper mount usage, proper delegation, proper error handling. Other fleet authors will study it. The code-analyst can reference the support fleet's own source when answering questions about how to build agents. The fleet documents itself.

**Optimize for the LLM, not the human.** Logs are no-op from a business logic perspective — they cannot harm the system. Verbosity is a challenge for humans reading terminal output, but the log-analyst is an LLM. It can process massive amounts of structured text. This means we should err on the side of logging too much rather than too little. Every message transition, every inference call, every delegation — log it all.

**The SHA is the trace ID.** The system timeline (ADR 055) already assigns content-addressed SHAs to every interaction. The `SubmissionId` propagates through all messages. There is no need for a separate tracing primitive — the SHA is built into the core. Every log line should carry it so the log-analyst can correlate across agents and reconstruct full execution history by joining logs with the timeline.

**Gaps in the support agent are the roadmap.** Whatever the log-analyst cannot answer reveals what is missing from the logging and observability story. Whatever the code-analyst cannot explain reveals gaps in documentation. The support fleet is a forcing function — its failures are direct improvement opportunities for the platform's supportability and observability domain model.

## Scope

### Day One

- Three agent containers: support, log-analyst, code-analyst
- Mount support in container runtime (`-v` flags from agent manifest)
- `vlinder support` subcommand as sugar for `vlinder fleet run`
- System prompt engineering for the five response categories
- Basic file search in agents (glob + read + send to inference)

### Deferred

- Embedding-based search in code-analyst (vector storage for smarter retrieval)
- Log streaming (tail -f equivalent for real-time analysis)
- Auto-installation of support fleet container images
- Caching of analysis results across sessions (KV storage in specialists)

## Consequences

- Vlinder ships with a useful fleet out of the box — `vlinder support` works on first install
- The support fleet validates the full stack: fleet deployment, delegation, mounts, inference
- Mount support in the container runtime benefits all container agents, not just the support fleet
- The five response categories give the orchestrator a clear decision framework instead of open-ended generation
- Python container agents establish a pattern for non-Rust agent development
- The support fleet's quality is bounded by the model — better local models mean better support
