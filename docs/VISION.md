# VlinderCLI

Building blocks for local AI agents.

### The Bet

Domain-specific agents with well-defined scope need accuracy and predictability,
not generalized capability. For that, you don't need frontier models or hyperscale
infrastructure. Small, efficient models — Phi-3, Mistral 7B, Qwen, Whisper,
Nomic Embed — on infrastructure you control, can deliver outsized value.

Local models are immutable artifacts. Same weights + same input = same output.
Execution traces become reproducible proofs. API providers can silently change
models; you cannot verify. Local models you can hash, replay, and debug.

### The Value Proposal

Agent systems today are messy. VlinderCLI aims to provide a domain specific vocabulary that makes them tractable: building blocks that map to how you think about agents.

- **Agent manifest** (`<name>-agent.toml` — or yaml, json): The agent, serialized, readable. Composed of building blocks.
- **Fleet manifest** (`<name>-fleet.toml` — or yaml, json): The orchestration layer for agents.
- **Runtime**: The execution engine that abstracts away the infrastructure.
- **Text boundaries**: Human-readable and extensible at every surface.

### Domain Model

The system is built around a few core concepts:

- **Agent**: A unit of behavior with declared requirements (models, services, mounts)
- **Model**: An inference or embedding capability (e.g., phi3, nomic-embed)
- **Fleet**: A deployment configuration for a collection of agents
- **Runtime**: Executes agents via queue-based message passing
- **Services**: Infrastructure capabilities exposed to agents (infer, embed, storage)

For the complete domain vocabulary, see [`DOMAIN_MODEL.md`](DOMAIN_MODEL.md).

 
### Who It's For

Agent builders who care about understanding:
- Homelab operators
- Startups watching their inference bill
- Enterprises with on-prem requirements
- Edge and IoT deployments


---

For implementation decisions, see `docs/adr/`.
For current work, see `TODO.md`.
