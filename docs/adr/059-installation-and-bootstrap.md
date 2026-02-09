# ADR 059: Installation, CI, and Release

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

### Inference is Ollama-only

Local inference goes through Ollama (ADR 060). There is no in-process llama.cpp backend. This simplifies the build — no native C++ toolchain required — but makes Ollama a prerequisite for any agent that uses LLMs.

## Decision

### 1. CI pipeline

CI runs on every push and pull request. It validates that the code compiles, passes tests, and meets quality standards — without requiring external services.

#### Test tiers

| Tier | Runs in CI | Requires | Invocation |
|------|-----------|----------|------------|
| Unit + integration (no services) | Yes | Nothing | `cargo test` |
| Ollama integration | No | Running Ollama | `cargo test -- --ignored` |
| NATS integration | No | Running NATS | `cargo test -- --ignored` |
| Container integration | No | Podman + built images | `cargo test -- --ignored` |

Tests that need external services are `#[ignore]`-annotated. `cargo test` skips them by default, so CI runs the full test suite without special configuration.

#### CI jobs

| Job | Purpose |
|-----|---------|
| **Lint** | `cargo fmt --check` + `cargo clippy -- -D warnings` |
| **Test** | `cargo test` (unit + integration, no external services) |
| **License** | `cargo deny check licenses` (no GPL/copyleft) |

Build dependencies: `protobuf-compiler` only. No cmake, no C++ toolchain (ADR 060 removed llama-cpp-2).

#### What CI does NOT do

- Run ignored tests (those need Ollama, NATS, or Podman)
- Build release artifacts (that's the release workflow)
- Deploy anything

### 2. Release pipeline

Triggered by pushing a version tag (`v*`). Builds release binaries for all supported targets and publishes them as a GitHub release.

#### Targets

| Target | Runner | Notes |
|--------|--------|-------|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | Standard Linux servers |
| `x86_64-apple-darwin` | `macos-13` | Intel Macs |
| `aarch64-apple-darwin` | `macos-latest` | Apple Silicon |

#### Artifact format

Each target produces a tarball: `vlinder-{target}.tar.gz` containing the `vlinder` binary. These are attached to the GitHub release. The install script downloads the appropriate tarball based on `uname -s` and `uname -m`.

### 3. Install script

A single `install.sh` that works on both macOS and Linux. The install script is the primary distribution channel.

```bash
curl -fsSL https://vlinder.dev/install.sh | sh
```

#### What it does

1. **Detect platform** — Darwin/Linux, x86_64/aarch64
2. **Check prerequisites** — print what's present and what's missing
3. **Install prerequisites** — NATS, Podman, Ollama via platform package manager (brew on macOS, apt/dnf on Linux)
4. **Download vlinder binary** — from the latest GitHub release
5. **Create directory structure** — `~/.vlinder/{agents,models,conversations,logs}`
6. **Write config** — `config.toml` with `distributed.enabled = true`, `queue.backend = "nats"`
7. **Start services** — NATS and vlinder daemon (launchd on macOS, systemd on Linux)
8. **Pull default model** — `phi3:latest` via Ollama
9. **Register default model** — `vlinder model add phi3`
10. **Print summary** — everything that was installed, started, and configured

#### Principles

- **Explicit**: every action is printed before it happens. No silent side effects.
- **Idempotent**: re-running skips what's already done (existing binaries, existing config, existing services).
- **Graceful degradation**: if Ollama isn't available, the script warns and continues. The system works for everything except LLM inference.
- **No surprises**: the script does not modify shell profiles, PATH, or environment variables beyond what it prints.

#### Platform-specific service management

| Concern | macOS | Linux |
|---------|-------|-------|
| Package manager | Homebrew | apt (Debian/Ubuntu) or dnf (Fedora/RHEL) |
| Service manager | launchd (plist in `~/Library/LaunchAgents/`) | systemd (user unit in `~/.config/systemd/user/`) |
| Auto-start | `RunAtLoad` in plist | `systemctl --user enable` |

#### What the install script does NOT do

- Configure network or firewall rules
- Build container images (agent images are pulled from a registry at deploy time)
- Modify shell profiles or PATH

### 4. The default model

The support fleet needs a model. The choice of which model is a configuration decision, not an agent decision. The agent manifests declare a logical alias (`default`), and the installer ensures that alias resolves to a real model.

The initial default is `phi3:latest` via Ollama. This is a pragmatic choice: small enough to run on most hardware, capable enough for triage and classification. Users can change it later via `vlinder model add`.

If Ollama is not available during install, the installer warns the user and skips model setup. The system is partially installed — everything except inference works. This is better than failing entirely.

### 5. Guard on `vlinder support`

`vlinder support` should never show a raw registration error. If the support fleet fails to deploy, the command should detect why and print actionable guidance:

```
Support fleet requires a registered model.
Run 'vlinder model add phi3' to register one manually.
```

This is a UI concern, not an architecture change. The registry validation is correct — the error message is the problem.

### 6. Prerequisites

Three external services form the minimum viable Vlinder installation:

| Prerequisite | Role | Required for |
|---|---|---|
| **NATS** | Message queue (distributed mode) | All agent execution |
| **Podman** | Container runtime | Running agents |
| **Ollama** | Inference and embedding backend (ADR 060) | Agents that use LLMs |

The installer checks for each and reports what's missing. NATS and Podman are hard requirements. Ollama is soft — the platform works without inference, but agents that need LLMs won't run.

## Scope

### Day One

- CI workflow: lint, test, license check (no external services)
- Release workflow: build binaries for 4 targets, publish GitHub release
- `install.sh`: cross-platform (macOS + Linux), prerequisite install, service setup, model bootstrap
- Improved error message on `vlinder support` when prerequisites are missing

### Deferred

- Homebrew formula
- DMG installer for macOS
- Linux package managers (apt, dnf) as standalone packages
- Windows support
- `vlinder doctor` — a diagnostic command that checks system health post-install

## Consequences

- `vlinder support` works immediately after installation — the bootstrapping gap is closed
- CI catches regressions without requiring external services
- Release artifacts are produced automatically on tag push
- No intermediate init step. Install → daemon → use.
- The error path for missing prerequisites becomes actionable guidance instead of raw errors
- The default model choice is centralized in the installer, not scattered across agent manifests
- Single install script works on both macOS and Linux
