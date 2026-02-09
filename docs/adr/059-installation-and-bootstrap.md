# ADR 059: Installation and Bootstrap

**Status:** Proposed

## Context

Running `vlinder support` fails immediately:

```
Failed to deploy fleet agent 'support': registration failed: model not registered: default
```

The support fleet's agents declare a model requirement (`default = "ollama://localhost:11434/phi3:latest"`). The registry validates model requirements at deploy time (ADR 050). No model is registered because the user never ran `vlinder model add`. The support agent — the one thing that should always work — is the first thing that breaks.

This is a bootstrapping problem. The platform has a chicken-and-egg dependency: agents need registered models, models need explicit registration, and the command that helps users figure out what went wrong (`vlinder support`) is itself an agent that needs a registered model.

The current installation story is "build from source and figure it out." There is no installer, no bootstrap step, no first-run experience. Every prerequisite is manual: install NATS, install Podman, install Ollama, pull a model, register the model, write config, then finally run agents. Missing any step produces a cryptic error.

### Distributed mode is the deployment target

Vlinder runs in distributed mode for all real usage. The daemon spawns worker processes that communicate via NATS queues (ADR 043). In-memory mode exists only for development and testing — it uses substring matching instead of token-based routing, has no consumer model, and hides bugs that surface in production.

This means:
- NATS is a hard prerequisite, not optional infrastructure
- `config.toml` must default to `queue.backend = "nats"` and `distributed.enabled = true`
- `vlinder daemon` is the standard way to run the platform
- The in-memory queue should never appear in a user-facing installation path

## Decision

### Installation via package manager or installer script

Vlinder is installed through standard distribution channels, not a CLI bootstrap command:

```bash
# macOS
brew install vlinder

# or
curl -fsSL https://vlinder.dev/install.sh | sh
```

The installer is responsible for the full bootstrap:

1. Install the `vlinder` binary
2. Install or verify prerequisites (NATS, Podman, Ollama)
3. Create `~/.vlinder/` directory structure (config, logs, agents, models, conversations)
4. Write `config.toml` with distributed mode enabled and NATS as queue backend
5. Pull the default model via Ollama
6. Register the default model in the registry

After installation, the user has a working system:

```bash
vlinder daemon    # start the platform
vlinder support   # ask for help
```

### The default model

The support fleet needs a model. The choice of which model is a configuration decision, not an agent decision. The agent manifests declare a logical alias (`default`), and the installer ensures that alias resolves to a real model.

The initial default is `phi3:latest` via Ollama. This is a pragmatic choice: small enough to run on most hardware, capable enough for triage and classification. Users can change it later via `vlinder model add`.

If Ollama is not available during install, the installer warns the user and skips model setup. The system is partially installed — everything except inference works. This is better than failing entirely.

### Guard on `vlinder support`

`vlinder support` should never show a raw registration error. If the support fleet fails to deploy, the command should detect why and print actionable guidance:

```
Support fleet requires a registered model.
Run 'vlinder model add phi3' to register one manually.
```

This is a UI concern, not an architecture change. The registry validation is correct — the error message is the problem.

### Prerequisites

Three external services form the minimum viable Vlinder installation:

| Prerequisite | Role | Required for |
|---|---|---|
| **NATS** | Message queue (distributed mode) | All agent execution |
| **Podman** | Container runtime | Running agents |
| **Ollama** | Inference and embedding backend (ADR 060) | Agents that use LLMs |

The installer checks for each and reports what's missing. NATS and Podman are hard requirements. Ollama is soft — the platform works without inference, but agents that need LLMs won't run.

### What the installer does NOT do

- Start NATS, Podman, or Ollama as services. The user manages service lifecycle (launchd, systemd, manual start).
- Build container images. Agent images are pulled from a registry, not built during install.
- Configure network or firewall rules.

## Scope

### Day One

- `install.sh` script: binary install, directory creation, config writing (distributed + NATS), prerequisite detection, model pull + registration
- Improved error message on `vlinder support` when prerequisites are missing
- Homebrew formula

### Deferred

- DMG installer for macOS
- Linux package managers (apt, dnf)
- Windows support
- `vlinder doctor` — a diagnostic command that checks system health post-install

## Consequences

- `vlinder support` works immediately after installation — the bootstrapping gap is closed
- No intermediate init step. Install → daemon → use.
- The error path for missing prerequisites becomes actionable guidance instead of raw errors
- The default model choice is centralized in the installer, not scattered across agent manifests
- Standard distribution channels (brew, curl) lower the barrier to entry
