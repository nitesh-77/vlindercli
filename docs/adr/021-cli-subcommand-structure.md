# ADR 021: CLI Subcommand Structure

## Status

Accepted

## Context

The CLI needs to support multiple types of runnable units: agents now, fleets soon, potentially more later. We need a consistent UX pattern.

## Decision

**Subcommand pattern: `vlinder <noun> <verb>`**

```
vlinder agent run [-p <path>]
vlinder fleet run [-p <path>]    # future
```

Design choices:

1. **Noun-verb structure** (`agent run`, not `run-agent`)
   - Groups related commands: `agent run`, `agent list`, `agent info`
   - Matches git/cargo/docker conventions

2. **Path via flag, not positional** (`-p ./path`, not `vlinder agent run ./path`)
   - Explicit over implicit
   - Leaves room for future positional args (e.g., input message)

3. **Default to current directory**
   - `vlinder agent run` in an agent directory just works
   - Mirrors `cargo build` behavior

4. **Separate REPL from command logic**
   - `commands/repl.rs` handles interactive loop
   - `commands/agent.rs` handles domain logic
   - Reusable for `fleet run`

## Consequences

- Consistent UX as CLI grows
- Adding `fleet run` follows established pattern
- `-p` flag convention carries across commands
- REPL behavior (welcome message, spinner, exit handling) is uniform
