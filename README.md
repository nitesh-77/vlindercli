# VlinderCLI

Building blocks for local AI agents.

## Motivation

Agent systems today are messy: tangled dependencies, opaque behavior, unpredictable costs. VlinderCLI provides a domain-specific vocabulary that makes agents tractable: building blocks that map to how you think about agents.

**The bet**: Domain-specific agents with well-defined scope need accuracy and predictability, not generalized capability. Small, efficient models (Phi-3, Mistral 7B, Nomic Embed) on infrastructure you control can deliver outsized value.

**Local models are reproducible**. Same weights + same input = same output. API providers can silently change models; you cannot verify. Local models you can hash, replay, and debug.

## Quick Start

```bash
# Build agents
just build-agents

# Run an agent
cargo run -- agent run -p agents/pensieve/
```

## Documentation

| Document | Description |
|----------|-------------|
| [Vision](docs/VISION.md) | What VlinderCLI is and who it's for |
| [Domain Model](docs/DOMAIN_MODEL.md) | Core types, traits, and their relationships |
| [Request Flow](docs/REQUEST_FLOW.md) | How requests travel through the system |
| [Timeline](docs/TIMELINE.md) | How versioned state and forking work |
| [Timeline Walkthrough](docs/TIMELINE_WALKTHROUGH.md) | Step-by-step grocery list example |
| [Bring Your Own Storage](docs/BRING_YOUR_OWN_STORAGE.md) | Integrating Postgres, Qdrant, etc. |
| [ADRs](docs/adr/) | Architecture Decision Records |

### Key ADRs

| ADR | Title |
|-----|-------|
| [018](docs/adr/018-protocol-first-architecture.md) | Protocol-First Architecture (queue-based message passing) |
| [020](docs/adr/020-agent-directory-structure.md) | Agent Directory Structure |
| [031](docs/adr/031-vlinderd-as-runtime-registry.md) | Registry as Runtime Authority |
| [055](docs/adr/055-time-travel-storage.md) | Version-Controlled Agent State |

## Project Structure

```
vlindercli/
├── src/
│   ├── domain/       # Abstract types and traits
│   ├── inference/    # LLM inference implementations
│   ├── embedding/    # Embedding implementations
│   ├── storage/      # Storage implementations
│   ├── queue/        # Message queue
│   └── runtime/      # WASM runtime and service handlers
├── agents/           # Example agents (pensieve, echo, etc.)
├── docs/
│   ├── VISION.md
│   ├── DOMAIN_MODEL.md
│   └── adr/          # Architecture Decision Records
└── tests/
```

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
