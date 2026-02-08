# VlinderCLI

AI agents that can time travel.

## Motivation

Agent systems are opaque. Something went wrong three turns ago and you'll never
know what. Vlinder makes every side effect (inference calls, storage writes,
delegation results) a content-addressed snapshot in a Merkle DAG. Fork a
timeline, replay from any point, diff what changed.

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


## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
