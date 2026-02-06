# ADR 019: Explicit Mounts Only

## Status

Superseded by ADR 046

WASI mount policy is moot — agents are OCI containers. Mount behavior is now standard Podman volume mounts declared in agent.toml.

## Context

ADR 016 introduced filesystem mounts with "magic" default behavior:

- If no `[[mounts]]` declared and `mnt/` exists → auto-add `mnt/ → /`
- Non-existent mount paths skipped with warning (graceful degradation)
- Runtime auto-creates `mnt/` on first execution

This creates problems:

1. **Filesystem-dependent behavior** - Same manifest behaves differently based on what directories exist on disk
2. **Hard to reason about** - "Does this agent have filesystem access?" requires checking disk state, not just the manifest
3. **Testing complexity** - Tests become environment-dependent
4. **Invisible coupling** - Agent capabilities depend on implicit state

## Decision

**Remove all magic. Mounts are fully explicit.**

```toml
# No [[mounts]] → no filesystem access
# Want filesystem? Declare it:
[[mounts]]
host_path = "mnt"
guest_path = "/"
mode = "rw"
```

**Behavior:**

| Manifest | Directory | Result |
|----------|-----------|--------|
| No `[[mounts]]` | — | No filesystem access |
| `[[mounts]]` declared | Path exists | Mount configured |
| `[[mounts]]` declared | Path missing | `LoadError::MountNotFound` |

**No auto-creation of directories.** If you declare a mount, ensure the directory exists.

**Fail-fast validation.** Mounts are resolved at load time. Missing paths cause immediate, clear errors rather than silent runtime failures.

## Consequences

- Agent behavior determined entirely by manifest, not filesystem state
- Predictable, testable, portable
- Slightly more verbose for agents that want filesystem access (must declare it)
- Aligns with Rust/Cargo philosophy: explicit over implicit
- Breaking change for agents relying on implicit `mnt/` mount
