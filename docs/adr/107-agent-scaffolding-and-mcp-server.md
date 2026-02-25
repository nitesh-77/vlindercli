# ADR 107: Agent Scaffolding and Development MCP Server

**Status:** Draft

## Context

The platform primitives work. Fleets deploy, agents delegate, inference
routes through provider hostnames, the conversation store captures
everything. But the path from "I want to build an agent" to "I have a
running agent" requires reverse-engineering the container contract from
existing examples. There is no scaffolding, no documentation the AI
assistant can consume, and no structural awareness of the developer's code.

Three gaps:

1. **No `vlinder agent new`.** The developer must hand-write agent.toml,
   Dockerfile, and the HTTP server skeleton by copying from existing agents.

2. **No machine-readable platform knowledge.** AI coding assistants (Claude
   Code, Gemini, Codex, Cursor, Lovable, Copilot, Windsurf — anything
   MCP-compatible) can't discover the container contract, provider
   hostnames, delegation protocol, or fleet structure without reading source
   code.

3. **No structural awareness.** The platform can't tell the developer that
   their agent.toml declares a service their code never calls, or that their
   fleet.toml references an agent that doesn't exist. This requires parsing
   the developer's code, not just the manifests.

## Decision

### `vlinder agent new`

`vlinder agent new <name>` scaffolds a Python agent project:

```
<name>/
  agent.toml          # manifest with sensible defaults
  Dockerfile          # python:3.12-alpine, COPY, EXPOSE 8080
  server.py           # HTTP skeleton (health + invoke endpoints)
  CLAUDE.md           # platform contract for AI assistants
  .mcp.json           # connects coding tools to vlinderd
```

Interactive prompts (or flags) select capabilities:

- `--infer openrouter` adds `[requirements.services.infer]` and wires
  the `openrouter.vlinder.local` URL into server.py
- `--delegate` adds delegation helpers (`runtime.vlinder.local`)
- `--kv` / `--vector` adds storage service wiring

The generated CLAUDE.md encodes the full agent development contract:
container protocol, provider hostnames, agent.toml schema, Dockerfile
conventions, delegation API, testing patterns.

The generated `.mcp.json` connects any MCP-compatible coding tool to
`vlinderd`:

```json
{
  "mcpServers": {
    "vlinder": {
      "type": "http",
      "url": "http://localhost:${VLINDER_PORT:-7100}/mcp"
    }
  }
}
```

The developer scaffolds the project, opens it in their tool of choice, and
the tool discovers the MCP server automatically. Between the CLAUDE.md
(static platform contract) and the MCP server (live structural awareness),
any coding agent has enough context to one-shot the agent implementation.

### MCP Server

Served by `vlinderd`, not a standalone binary. The MCP server is another
capability of the daemon, alongside the registry, harness, runtime, and
provider services.

The MCP server is backed by a **support agent fleet** — the platform's
developer tooling runs on the platform itself. MCP requests route to
specialist agents that reason over four knowledge sources:

**1. Platform source (vlindercli).** Tree-sitter parses and indexes the
vlindercli Rust source. This index is embedded at build time — same
version, same commit as the binary. The platform's own code is the source
of truth for the container contract, provider hostnames, message types, and
delegation protocol. No documentation to drift.

**2. Runtime state (~/.vlinder/).** The support fleet reads the `.vlinder/`
directory: conversation payloads, logs, agent state. Today, debugging a
failed agent means manually reading `~/.vlinder/conversations/*/payload`
and `~/.vlinder/logs/`. The fleet makes this queryable — any coding tool
can ask "what happened?" instead of the developer spelunking through files.
The platform dogfoods its own observability surface.

**3. Agent code.** The developer's agent code, parsed live from disk with
tree-sitter. Grammars ship for the languages agents are written in
(Python, TypeScript, Go, etc.) plus TOML for manifests. This gives
cross-referencing ability: the fleet knows what the platform offers (from
source #1) and what the developer's agent uses (from their code + TOML),
and can bridge the two — flagging mismatches, suggesting wiring, answering
structural questions.

**4. Platform docs.** ADRs, README, and project documentation — embedded
alongside the source. The fleet knows not just *what the code does* but
*why it was designed that way*. A coding tool can ask "how does delegation
work?" and get both the implementation and the rationale.

### Fallback: CLAUDE.md + SQLite RAG

The support fleet requires a running daemon and healthy agents. When
either is unavailable, the fallback path provides the same knowledge
through static artifacts:

- **CLAUDE.md** — the platform contract, checked into every scaffolded
  project. Any coding tool can read it directly, no daemon needed.
- **SQLite RAG** — embedded vectors over platform source, docs, and
  runtime state. The same data the fleet agents query, pre-indexed into
  the existing SQLite store. Direct access, no agent orchestration.

Three tiers of availability:

1. **Fleet running** — full reasoning, delegation, observable agent
   conversations
2. **Daemon running, fleet down** — MCP falls back to SQLite RAG queries
3. **Nothing running** — CLAUDE.md in the repo

The fleet doesn't add new data. It adds reasoning over data that's already
queryable through SQLite. The fallback degrades gracefully from
intelligence to retrieval to static docs.

**Tool-agnostic.** MCP is an open protocol. Any tool that speaks MCP gets
full platform awareness: Claude Code, Gemini, Codex, Cursor, Lovable,
Copilot, Windsurf, and whatever ships next. The `.mcp.json` in the
scaffolded project is the universal on-ramp.

### The one-shot flow

1. `vlinder agent new myagent` — scaffolds project with manifests, code
   skeleton, CLAUDE.md, and .mcp.json
2. Developer opens the project in any MCP-compatible coding tool
3. Tool reads CLAUDE.md (static contract) and connects to vlinderd's MCP
   server (live awareness)
4. Developer describes what the agent should do
5. Coding tool has enough context to produce a working agent in one pass

## Consequences

- Every scaffolded agent project ships with `.mcp.json` — MCP becomes the
  default integration path, not a bolt-on
- `vlinderd` gains an MCP endpoint, adding tree-sitter as a build
  dependency
- Platform source and docs are embedded in the binary, increasing binary
  size
- The `.vlinder/` directory becomes a first-class API surface, not just
  files on disk
- The support fleet is the platform's first dogfood consumer of fleets and
  delegation — bugs in the fleet primitives surface here first
- Graceful degradation: fleet → SQLite RAG → CLAUDE.md. No single point of
  failure locks the developer out of platform knowledge
- The support fleet's own conversations are observable in `~/.vlinder/`,
  making it possible to debug the debugger
