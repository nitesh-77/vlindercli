# VlinderCLI

AI agents that can time travel.

### The Bet

Agent systems are opaque. Something went wrong three turns ago and you'll never
know what. VlinderCLI makes every side effect (inference calls, storage writes,
delegation results) a content-addressed snapshot in a Merkle DAG. Fork a
timeline, replay from any point, diff what changed.

### Why It Matters

- **Reason**: A domain model that maps to how you think about agent infrastructure.
- **Debug**: Something broke. Travel back, see exactly what the agent saw, what it did, and why. 
- **Experiment**: Fork a timeline, try a different prompt or model, compare the outcomes.
- **Prove**: Every side effect has a hash. Auditable by construction, not by convention.
- **Time Travel**: Checkout any point in history, repair what is broken, keep everything else, and promote the fixed timeline.

### Domain Model

The system is built around a few core concepts:

- **Agent**: A unit of behavior with declared requirements (models, services, mounts)
- **Model**: An inference or embedding capability (e.g., claude-sonnet, nomic-embed)
- **Fleet**: A deployment configuration for a collection of agents
- **Runtime**: Executes agents via queue-based message passing
- **Services**: Infrastructure capabilities exposed to agents (infer, embed, storage)
- **Timeline**: A git repository of every side effect — each message is a commit, forkable and diffable
- **State**: Versioned agent state (values, snapshots, state commits) linked to the timeline via content-addressed hashes
- **Route**: The full protocol trace through a fleet — every agent stop from user input to user output

For the complete domain vocabulary, see [`DOMAIN_MODEL.md`](DOMAIN_MODEL.md).

### Who It's For

Developers who want to understand what their agents are doing, and prove it.
- Teams building AI features that need reproducibility and auditability
- Startups who need to control inference costs and infrastructure
- Anyone who's debugged an agent by staring at logs and wants something better


---

For implementation decisions, see `docs/adr/`.
For current work, see `TODO.md`.
