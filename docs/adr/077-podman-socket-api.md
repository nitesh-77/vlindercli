# ADR 077: Podman Socket API

## Status

Accepted (validated in 66573d8)

## Context

All Podman interactions shell out via `Command::new("podman")` in `PodmanCli`. This means:

- Untyped string responses require fragile parsing (port output, version strings)
- Subprocess overhead per call (fork+exec)
- Error messages are raw stderr strings
- No structured error codes

Podman exposes a REST API over a Unix socket (`/v5.0.0/libpod/...`) with typed JSON responses. The existing `Podman` trait was designed for exactly this kind of swap — `ContainerRuntime` holds `Box<dyn Podman>`.

## Decision

**Add `PodmanSocket`, a new `Podman` trait implementation that talks to the Podman REST API over a Unix socket.** Selection is config-driven: `runtime.podman_socket` can be `"auto"` (probe standard paths), `"disabled"` (force CLI), or an explicit path.

The Unix transport uses ureq v3's `Connector`+`Transport` traits to route HTTP requests over `std::os::unix::net::UnixStream`, reusing ureq's HTTP framing and JSON handling.

Socket discovery probes in order:
1. `$XDG_RUNTIME_DIR/podman/podman.sock` (standard rootless)
2. `$HOME/.local/share/containers/podman/machine/podman.sock` (macOS Podman Machine)
3. `/run/podman/podman.sock` (rootful)

Falls back to `PodmanCli` if no socket is found.

## Consequences

- Typed JSON responses replace fragile string parsing for container lifecycle operations
- No new crate dependencies — ureq (already used) handles HTTP framing over the custom transport
- `PodmanCli` remains as fallback — zero behavioral change when socket is unavailable
- ureq bumped from v2 to v3 to access the `Transport` trait (all existing call sites updated)

## Deferred

- Windows named pipe transport (Podman on Windows uses `\\.\pipe\podman-machine-default`)
- Connection pooling for the Unix socket (currently one connection per request)
- Streaming container logs via the socket API
