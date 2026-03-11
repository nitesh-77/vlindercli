# VlinderCLI

**Debug and repair AI agents.**

### The Problem

Traditional software gets more deterministic toward production — CI, observability, and canary deploys exist to keep it that way. AI agents invert this. The model is stochastic: same input, different output. Human input is unconstrained. State accumulates unpredictably. Production becomes the *most* non-deterministic stage in the lifecycle — the exact opposite of what every existing tool assumes.

Developers can detect AI failures. They cannot easily replay and repair them.

Today: agent fails → inspect logs → reconstruct prompt → rerun locally → test new prompt → redeploy.

With VlinderCLI: agent fails → rewind to failure → inspect state → test fix → replay.

### The Solution

VlinderCLI captures every side effect (inference calls, storage writes, delegation results) as a content-addressed snapshot. Fork a session, replay from any point, diff what changed.

- **Rewind** — go back to the exact step where the failure happened
- **Experiment** — branch execution and test alternative prompts, models, or code
- **Repair** — correct downstream actions and replay the workflow
- **Deploy** — works with existing agent frameworks and cloud infrastructure

### Domain Model

The system is built around a few core concepts:

- **Agent**: A unit of behavior with declared requirements (models, services, mounts)
- **Model**: An inference or embedding capability (e.g., claude-sonnet, nomic-embed)
- **Fleet**: A group of cooperating agents with delegation
- **Session**: An independent conversation with content-addressed history
- **Services**: Infrastructure capabilities exposed to agents (infer, embed, storage)
- **State**: Versioned agent state linked to the session via content-addressed hashes

For the complete domain vocabulary, see [`DOMAIN_MODEL.md`](DOMAIN_MODEL.md).

### Who It's For

- Teams building AI agents that need to debug failures quickly
- Developers who want to understand exactly what their agents did, and why
- Anyone who's debugged an agent by staring at logs and wants something better

---

For implementation decisions, see `docs/adr/`.
For current work, see `TODO.md`.
