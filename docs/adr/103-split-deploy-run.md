# ADR 103: Split Deploy and Run

**Status:** Accepted

## Context

`vlinder agent run` does two things: deploys the agent manifest to the
registry, then starts a REPL that invokes the agent via the queue.

This coupling breaks re-runs. After time-traveling with
`vlinder timeline checkout`, running the agent again fails because the
agent is already registered and the deploy step returns `DuplicateName`.

Deploy is a registry operation. Run is a harness operation. They have
different lifecycles — you deploy once, run many times.

## Decision

Split into two commands:

### Deploy

```
$ vlinder agent deploy
Deployed: todoapp (http://127.0.0.1:9000/agents/todoapp)

$ vlinder agent deploy --path ../other-agent
Deployed: other-agent (http://127.0.0.1:9000/agents/other-agent)
```

`--path` defaults to the current directory. File I/O (reading
`agent.toml`, resolving paths) is a CLI concern. The registry receives
the fully-resolved `AgentManifest`. Idempotent (ADR 102).

### Run

```
$ vlinder agent run todoapp
Connected to todoapp
> add milk
```

Takes an agent name, not a path. Looks up the agent in the registry,
starts the REPL, invokes via the queue. Does not deploy.

If the agent is not registered, fails with a clear error:
`agent 'todoapp' not found — deploy it first with: vlinder agent deploy`

### deploy_from_path removed from Harness

`CliHarness::deploy_from_path()` combined file loading with registration.
With the split, the CLI command handles file loading and the registry
handles registration. The harness is only responsible for invoke, poll,
and session management (ADR 101).

## Consequences

- `vlinder agent run` works after time travel without re-deploying.
- Deploy and run have independent lifecycles.
- `deploy_from_path()` is removed from the harness.
- `vlinder fleet run` follows the same pattern: deploy all agents first,
  then run.
