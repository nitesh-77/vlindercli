# VlinderCLI

**Debug and repair AI agents.**

When AI agents fail, teams spend hours reconstructing what happened — tracing prompts, logs, and external actions to find the root cause. VlinderCLI captures every decision your agent makes, so you can rewind to the exact failure, inspect state, test a fix, and replay.

## Quick Start

```bash
# Build agents
just build-agents

# Deploy and run an agent
cargo run -- agent deploy -p agents/todoapp/
cargo run -- agent run todoapp
```

## How it works

VlinderCLI makes every side effect (inference calls, storage writes, delegation results) a content-addressed snapshot. Fork a session, replay from any point, diff what changed.

- **Rewind** — go back to the exact step where the failure happened
- **Experiment** — branch execution and test alternative prompts, models, or code
- **Repair** — correct downstream actions and replay the workflow
- **Deploy** — works with existing agent frameworks and cloud infrastructure

```
$ vlinder agent run todoapp
> add buy milk
> add buy eggs
> add buy carrots    # wrong — should be bread

$ vlinder session fork wired-pig-543e \
    --from a1b2c3d4 --name fix-groceries

$ vlinder agent run todoapp --branch fix-groceries
Resuming from state a1b2c3d4…
> list
1. buy milk
2. buy eggs          # rewound past carrots
> add buy bread      # fixed

$ vlinder session promote fix-groceries
```

## Documentation

| Document | Description |
|----------|-------------|
| [Vision](docs/VISION.md) | What VlinderCLI is and who it's for |
| [Domain Model](docs/DOMAIN_MODEL.md) | Core types, traits, and their relationships |
| [Request Flow](docs/REQUEST_FLOW.md) | How requests travel through the system |
| [ADRs](docs/adr/) | Architecture Decision Records |

Full docs at [docs.vlindercli.dev](https://docs.vlindercli.dev).

## License

Apache 2.0 — see [LICENSE](LICENSE).
