# ADR 091: Sidecar Bridge

## Status

Accepted

## Context

Each container agent runs as a Podman container. The platform dispatches
work to it (POST /handle), mediates all external interactions through the
QueueBridge, and records every side effect into the DAG. The execution
engine — dispatch loop, bridge, state tracking — lived inside vlinderd as
an in-process thread per agent.

This coupling created two problems:

1. **vlinderd is the execution engine.** Dispatch, bridge wiring, and
   state tracking run on threads inside the daemon. If the daemon
   restarts, all in-flight invocations die. The daemon can't be updated
   without killing running agents.

2. **Identity requires infrastructure.** When the sidecar eventually
   needs to serve HTTP endpoints (OpenAI-compatible proxies for standard
   SDKs), the agent must reach a bridge process. If that bridge is on
   the host, it needs per-container ports, signed tokens (ADR 084/085),
   or identity files to know which agent is calling. All of these add
   infrastructure.

### The dispatch loop already solved identity

The current dispatch loop doesn't have an identity problem because the
platform initiates the connection. It holds the QueueBridge with the
agent's full context — agent_id, submission, session, state cursor.
Identity is implicit in the call direction.

The insight: if the bridge process runs *next to* the agent in the same
network namespace, identity is solved by construction. The agent talks to
localhost; localhost is the bridge. No shared endpoint, no tokens.

## Decision

**The sidecar is a standalone binary (`vlinder-sidecar`) deployed as an
OCI container alongside the agent container in a Podman pod.**

### Podman pod

```
vlinderd                              Podman Pod (shared localhost)
┌──────────────┐    ┌─────────────────────────────────────────┐
│ Container    │    │  ┌───────────┐    ┌───────────────────┐ │
│ Runtime      │───→│  │  Agent    │    │    Sidecar        │ │
│ (creates pod)│    │  │ container │←──→│ (vlinder-sidecar) │ │
│              │    │  └───────────┘    └────────┬──────────┘ │
└──────────────┘    │                           │             │
                    └───────────────────────────┼─────────────┘
                                                │
                                           NATS + gRPC
                                          (host services)
```

The agent and sidecar share a network namespace (localhost). The sidecar
connects to NATS and the gRPC registry/state services on the host via
`host.containers.internal`.

### Sidecar binary crate

`crates/vlinder-sidecar/` — depends on vlinder-core + vlinder-nats,
does NOT depend on vlinderd. Keeps the binary lean.

Configuration via environment variables (12-factor):

| Variable | Purpose |
|---|---|
| `VLINDER_AGENT` | Agent name (queue subscription key) |
| `VLINDER_NATS_URL` | NATS server URL |
| `VLINDER_REGISTRY_URL` | gRPC registry service URL |
| `VLINDER_STATE_URL` | gRPC state service URL |
| `VLINDER_CONTAINER_PORT` | Agent container HTTP port (default 8080) |
| `VLINDER_IMAGE_REF` | OCI image reference (diagnostics) |
| `VLINDER_IMAGE_DIGEST` | Content-addressed digest (diagnostics) |

### Sidecar lifecycle

1. Parse config from env vars
2. Connect to NATS (with DAG recording via RecordingQueue)
3. Connect to gRPC registry, fetch agent metadata
4. Wait for agent container readiness (`GET localhost:{port}/health`)
5. Enter dispatch loop: poll invoke/delegate queues, dispatch to agent,
   send complete messages
6. On container death: exit loop, pod dies, runtime restarts it

### Pod lifecycle (ContainerRuntime)

The runtime creates pods, not containers:

1. `podman pod create --name vlinder-{agent}`
2. `podman create --pod {pod_id} {agent_image}` (agent container)
3. `podman create --pod {pod_id} --env ... vlinder-sidecar` (sidecar container)
4. `podman pod start {pod_id}`

The runtime no longer holds threads, bridges, or dispatch logic. It only
manages compute lifecycle. The Podman trait gained pod operations
(`pod_create`, `container_in_pod`, `pod_start`, `pod_stop_and_remove`);
single-container methods were removed.

### What moves out of vlinderd

- `dispatch.rs` (dispatch state machine) → `crates/vlinder-sidecar/src/dispatch.rs`
- `pod.rs` (Sidecar struct, bridge wiring) → `crates/vlinder-sidecar/src/sidecar.rs`
- Thread management, JoinHandle tracking → deleted (pods are external processes)

### Network: host.containers.internal

Sidecar env vars use `host.containers.internal` for NATS, registry, and
state service URLs. This is Podman's standard hostname for reaching the
host from inside a container/pod. Works reliably on macOS Podman Machine;
on native Linux rootless Podman it requires `--network slirp4netns` or
pasta networking.

## Consequences

- vlinderd becomes a pod orchestrator — no execution engine, no threads
- Agent lifecycle is decoupled from daemon lifecycle
- Identity is solved by construction — no tokens, no identity files
- DAG recording works unchanged (RecordingQueue in sidecar)
- Standard SDK support becomes possible: sidecar can serve
  OpenAI-compatible endpoints on localhost (future increment)
- Podman pod support required — first use of pods in the platform
- Sidecar is shipped as an OCI container image (`just build-sidecar`)

## Deferred

- OpenAI-compatible HTTP proxy endpoints on the sidecar
- Dead pod detection (currently relies on ensure_containers on next tick)
- Retry logic in sidecar (container death exits the loop)
- Linux rootless networking validation
