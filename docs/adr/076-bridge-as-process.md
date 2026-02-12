# ADR 076: Bridge as Independent Process

## Status: Draft

## Context

The bridge routes agent SDK calls (kv, infer, embed, delegate, wait) to platform services via the queue. Today it runs as a background thread inside the daemon, one thread per container, each on an OS-assigned port.

This works for local mode but breaks the distributed model. Every other moving part — registry, workers, runtime — is an independent process connected via NATS. The bridge is the exception: it's embedded in the runtime process and cannot be scaled, deployed, or failed independently.

Problems with the thread-per-container model:

1. **N containers = N HTTP servers = N ports.** Each container gets its own bridge thread on a unique port. Operationally noisy, hard to firewall.
2. **Not independently scalable.** The bridge can't be moved to a different machine or scaled horizontally.
3. **Per-invocation state is implicit.** Each bridge thread holds `RwLock<InvokeMessage>`, `current_state`, and a sequence counter. The state is scoped by being the only thread serving that container — not by any explicit key.
4. **Runtime process does two jobs.** Container lifecycle (pool, dispatch) and serving SDK calls (bridge HTTP + queue routing) are unrelated concerns sharing a process boundary.

## Decision

The bridge becomes its own process, connected to NATS like every other component.

**Single HTTP endpoint.** One bridge process serves all containers on one port. Containers connect to the same `VLINDER_BRIDGE_URL` instead of N different ports.

**Session-keyed invocation context.** The container already sends `X-Vlinder-Session` on every SDK request. The bridge maintains a `HashMap<SessionId, InvocationContext>` to multiplex state:

```rust
struct InvocationContext {
    submission: SubmissionId,
    session: SessionId,
    agent_id: ResourceId,
    kv_backend: Option<ObjectStorageType>,
    vec_backend: Option<VectorStorageType>,
    model_backends: HashMap<String, String>,
    sequence: SequenceCounter,
    current_state: Option<String>,
}
```

**Context lifecycle.** The runtime publishes an `Invoke` message to the queue. The bridge learns about new invocations either:
- By receiving an explicit "register session" message from the runtime before the container starts, or
- By resolving the session's agent config from the registry on first SDK call.

Contexts are evicted on completion (runtime sends a signal) or after a TTL.

**No changes to AgentBridge trait.** The trait (ADR 074) stays the same. `HttpBridge` (the queue-backed implementation) stays the same. Only the HTTP transport layer changes — from thread-per-container to multiplexed-by-session.

## Consequences

- **Independently deployable.** `vlinder bridge` runs as its own process. In local mode, the daemon can still spawn it as a child process for convenience.
- **One port.** All containers connect to the same bridge endpoint. Simplifies networking, firewall rules, and container configuration.
- **Scalable.** Multiple bridge instances can run behind a load balancer, each connected to NATS.
- **Runtime simplification.** `ContainerRuntime` no longer builds or manages bridge servers. It starts containers with a known bridge URL and dispatches invokes via the queue.
- **State becomes explicit.** Per-invocation state is keyed by session ID in a map, not implied by thread identity. This is more honest and debuggable.
- **New process to operate.** In distributed mode this is expected (everything is a process). In local mode, the daemon manages it.
