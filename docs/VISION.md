# VlinderCLI

Building blocks for local AI agents.

### The Bet

Domain-specific agents with well-defined scope need accuracy and predictability,
not generalized capability. For that, you don't need frontier models or hyperscale
infrastructure. Small, efficient models — Phi-3, Mistral 7B, Qwen, Whisper,
Nomic Embed — on infrastructure you control, can deliver outsized value.

### The Value Proposal

Agent systems today are messy. VlinderCLI aims to provide a domain specific vocabulary that makes them tractable: building blocks that map to how you think about agents.

- **Agent manifest** (`<name>-agent.toml` — or yaml, json): The agent, serialized, readable. Composed of building blocks.
- **Fleet manifest** (`<name>-fleet.toml` — or yaml, json): The orchestration layer for agents.
- **Runtime**: The execution engine that abstracts away the infrastructure.
- **Text boundaries**: Human-readable and extensible at every surface.

### Separation of Concerns

The **agent manifest** declares what an agent is and how it runs. It is composed of:
- **Identity**: Name and description — how the agent identifies itself
- **Code**: The agent's logic
- **Packaging**: How the code is bundled (WASM, Firecracker, container)
- **Models**: Which models the agent needs
  - phi3 (generation)
  - nomic-embed (embedding)
  - whisper (transcription)
  - llava (vision)
- **Prompts**: Behavior, personality, constraints
- **Services**: Which runtime services the agent needs (fetch, store, embed, infer)
- **Mounts**: Filesystem access and persistence

The **fleet manifest** declares where and how to deploy a collection of agents.
Which deployment target, which storage backends, which inference providers.
Today, the fleet manifest is implicit — sensible defaults, no spec file needed.
It becomes explicit when deployment needs parameterization.
<!-- TODO: Fleet manifest spec not yet modeled -->

**Runtime** executes agents and satisfies their requirements. It owns the
hard problems: model lifecycle, inference scheduling, storage, permissions.
Different runtimes can implement the same interface for different targets.

 
### Who It's For

Agent builders who care about understanding:
- Homelab operators
- Startups watching their inference bill
- Enterprises with on-prem requirements
- Edge and IoT deployments


---

For implementation decisions, see `docs/adr/`.
For current work, see `TODO.md`.
