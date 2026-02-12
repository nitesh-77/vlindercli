# ADR 073: Container Metadata Capture

## Status

Accepted

## Context

ADR 071 established `ContainerRuntimeInfo` as part of `ContainerDiagnostics`:

```rust
pub struct ContainerRuntimeInfo {
    pub engine_version: String,
    pub image_ref: String,
    pub image_digest: String,
    pub container_id: String,
}
```

All four fields currently return `"unknown"` via `ContainerDiagnostics::placeholder()`. The data exists — Podman returns it and `ContainerRuntime` already captures `container_id` — but nothing flows it into the diagnostics on the `CompleteMessage`.

### The correctness problem

`ensure_container()` checks its HashMap — if the agent name is present, it assumes the container is alive and returns the cached port. But the container may no longer exist. Today, the dispatch fails and the error propagates to the user. There is no recovery.

Three scenarios cause this:

1. **Developer rebuilds the image.** The normal development loop: `podman build`, `podman stop` the old container, manually `podman run` to test, then return to the daemon. The daemon's HashMap still points to the old (now dead) container. This is the most common case.

2. **Container crash.** The agent process exits unexpectedly. The runtime doesn't know — it dispatches to a dead port.

3. **Podman restart.** A system-level restart (package upgrade, service restart) kills all containers. The runtime's HashMap is entirely stale.

In all three cases, the container the runtime thinks it owns is gone. Dispatch fails. Currently there is no eviction or retry — the daemon must be restarted.

Note on image tags: the developer uses mutable tags like `localhost/todoapp:latest` during development. Asking them to update a pinned digest in the manifest after every `podman build` is terrible UX. The platform must handle both mutable tags and pinned digests — the choice is a runtime concern, not a registration concern (see section 4).

Metadata is only trustworthy if it describes the container that **actually served the request**. This requires the runtime to detect when a container is gone and re-capture metadata on restart.

### What we control

The platform owns the container lifecycle. The agent doesn't know its own `container_id`, `image_digest`, or `engine_version`. These are platform-observed facts — consistent with the provenance model from ADR 071.

The agent does prove liveness implicitly: a successful `/invoke` response means the container was alive for that request. A failed dispatch is the signal that the container is gone.

## Decision

### 1. Capture metadata at container start, verify by dispatch outcome

Metadata is captured fresh every time `ensure_container()` starts a new container. The dispatch outcome (success/failure of the HTTP POST to `/invoke`) is the liveness signal. On dispatch failure, the runtime evicts the stale container and retries with a fresh one.

No separate health check call. No polling. The normal request path is the verification.

#### Sequence: First invocation (cold start)

```
Harness          ContainerRuntime          Podman            Agent
  │                    │                     │                 │
  │─── InvokeMessage ──►                     │                 │
  │                    │                     │                 │
  │                    │── podman run -d ───►│                 │
  │                    │◄── container_id ────│                 │
  │                    │                     │                 │
  │                    │── podman image ────►│                 │
  │                    │   inspect (digest)  │                 │
  │                    │◄── sha256:a80c... ──│                 │
  │                    │                     │                 │
  │                    │  store on ManagedContainer:           │
  │                    │  { container_id, image_ref,           │
  │                    │    image_digest }                     │
  │                    │                     │                 │
  │                    │── POST /invoke ─────────────────────► │
  │                    │◄── 200 OK ──────────────────────────  │
  │                    │                     │                 │
  │                    │  build ContainerDiagnostics           │
  │                    │ from ManagedContainer + engine_version│
  │                    │                     │                 │
  │◄─ CompleteMessage ─│                     │                 │
  │  (real diagnostics)│                     │                 │
```

Which image reference is passed to `podman run` depends on the image policy (section 4): `agent.executable` (the tag) in `mutable` mode, or `agent.image_digest` (pinned hash) in `pinned` mode. After `podman run`, `podman image inspect` always runs on the same reference to resolve the content-addressed digest for diagnostics — this is the ground truth of what actually ran.

#### Sequence: Warm invocation (container alive)

```
Harness          ContainerRuntime          Agent
  │                    │                     │
  │─── InvokeMessage ──►                     │
  │                    │                     │
  │                    │  HashMap hit ✓      │
  │                    │  (container cached) │
  │                    │                     │
  │                    │── POST /invoke ────►│
  │                    │◄── 200 OK ────────  │
  │                    │                     │
  │                    │  build ContainerDiagnostics
  │                    │  from cached ManagedContainer
  │                    │                     │
  │◄─ CompleteMessage ─│                     │
  │   (real diagnostics)                     │
```

No extra Podman calls on the warm path. Metadata was captured at start and is valid as long as dispatch succeeds.

#### Sequence: Container died, dispatch fails, retry

```
Harness          ContainerRuntime          Podman            Agent
  │                    │                     │                 │
  │─── InvokeMessage ──►                     │                 │
  │                    │                     │                 │
  │                    │  HashMap hit ✓      │              ✗ (dead)
  │                    │  (stale entry)      │                 │
  │                    │                     │                 │
  │                    │── POST /invoke ────► ✗ connection refused
  │                    │                     │                 │
  │                    │  evict stale entry  │                 │
  │                    │  log: container.evicted               │
  │                    │                     │                 │
  │                    │── podman run -d ───►│                 │
  │                    │◄─ new_container_id ─│                 │
  │                    │                     │                 │
  │                    │── podman image ────►│                 │
  │                    │   inspect (digest)  │                 │
  │                    │◄── sha256:... ──────│                 │
  │                    │                     │                 │
  │                    │  store fresh ManagedContainer         │
  │                    │                     │                 │
  │                    │── POST /invoke ─────────────────────► │
  │                    │◄── 200 OK ──────────────────────────  │
  │                    │                     │                 │
  │◄─ CompleteMessage ─│                     │                 │
  │   (fresh diagnostics, new container_id)  │                 │
```

The dispatch failure triggers eviction. The retry starts a fresh container and re-captures all metadata. In `mutable` mode, this picks up rebuilt images automatically. In `pinned` mode, the same exact image runs again.

### 2. Where each field comes from

| Field | Source | When captured |
|-------|--------|---------------|
| `engine_version` | `podman version --format '{{.Client.Version}}'` | Once at `ContainerRuntime::new()` |
| `image_ref` | The reference passed to `podman run` (`agent.executable` or `agent.image_digest` depending on policy) | At `ensure_container()` start path |
| `image_digest` | `podman image inspect <image> --format '{{.Digest}}'` | At `ensure_container()` start path |
| `container_id` | stdout of `podman run` | At `ensure_container()` start path |

`engine_version` is cached once because a Podman upgrade requires a service restart — the version won't change mid-process. If the `podman version` call fails at startup (Podman not installed), degrade to `"unknown"`.

`image_digest` is always resolved via `podman image inspect` at container start. This is the ground truth — what Podman actually ran. In `pinned` mode, it should match the registration-time digest; in `mutable` mode, it reflects whatever the tag currently points to.

### 3. Dispatch-failure eviction replaces health checks

No separate liveness mechanism. The dispatch HTTP POST to `/invoke` is the liveness check:

- **Success**: container is alive, cached metadata is valid.
- **Failure** (connection refused, timeout): container is dead. Evict from HashMap, stop/rm the old container, start a fresh one, retry the dispatch once.

This avoids:
- Extra Podman subprocess calls before every dispatch
- Extra HTTP round-trips to `/health` before every dispatch
- The complexity of a polling or timer-based health check

The trade-off is that the first dispatch to a dead container fails and must retry. This adds latency to the recovery path only — the happy path has zero overhead.

### 4. Image policy: store both, switch at runtime

Registration always resolves the image reference and stores **both forms**. Storage is cheap.

```rust
pub struct Agent {
    /// What the user wrote: tag or pinned ref.
    pub executable: String,
    /// Resolved content-addressed digest (from podman image inspect at registration).
    pub image_digest: String,
    // ...
}
```

```
registration:
  executable    = "localhost/todoapp:latest"     (original, preserved)
  image_digest  = "sha256:a80c4f17..."           (resolved via podman image inspect)
```

Registration is uniform — always resolve, always store both. No branching.

The config is a **runtime** concern. It determines which reference to pass to `podman run`:

```toml
[runtime]
image_policy = "mutable"    # development (default)
# image_policy = "pinned"   # production
```

| Policy | `podman run` receives | Effect |
|--------|----------------------|--------|
| `mutable` | `agent.executable` (the tag) | Rebuilds picked up automatically |
| `pinned` | `agent.image_digest` (content-addressed) | Exact image from registration |

The runtime switches on one field — everything else (eviction, diagnostics, metadata capture) is identical.

Diagnostics always have both values regardless of policy.

In `mutable` mode:

```toml
[container.runtime]
image_ref = "localhost/todoapp:latest"
image_digest = "sha256:b91d5e28..."    # may differ from registration if rebuilt
```

In `pinned` mode:

```toml
[container.runtime]
image_ref = "sha256:a80c4f17..."       # the pinned digest from registration
image_digest = "sha256:a80c4f17..."    # matches — same image guaranteed
```

`image_ref` is what was passed to `podman run` — a tag in `mutable` mode, a digest in `pinned` mode. `image_digest` is the content hash of what actually ran (from `podman image inspect` at container start). In `pinned` mode these always match the registration-time values. In `mutable` mode, `image_digest` reflects whatever the tag currently points to — it may differ from the registration-time value if the image was rebuilt.

#### Default: `mutable`

Most users encounter Vlinder during development first. Requiring re-registration after every `podman build` is a barrier to adoption. `mutable` gets out of the way. Users who need production guarantees opt into `pinned`.

### 5. Data flow: ManagedContainer → ContainerDiagnostics → CompleteMessage

`ManagedContainer` gains the metadata fields. The runtime builds `ContainerDiagnostics` from them in `tick()` when sweeping finished tasks.

`create_reply_with_state()` on `InvokeMessage` currently hard-codes `ContainerDiagnostics::placeholder(0)`. It gains a sibling: `create_reply_with_diagnostics(payload, state, diagnostics)` that accepts a real `ContainerDiagnostics`.

`RunningTask` gains `started_at: Instant` for duration measurement, and a reference to the agent name (already has it via the HashMap key) to look up the `ManagedContainer` metadata at completion time.

## Consequences

- `ContainerRuntimeInfo` in `diagnostics.toml` shows real Podman metadata instead of `"unknown"`
- `image_policy` config makes the development/production trade-off explicit — no hidden behavior
- `mutable` (default): developers rebuild and iterate without re-registering — tags re-resolve on container start
- `pinned`: agents are immutable with content-addressed images — deterministic execution guaranteed
- Dead containers are detected and restarted automatically (one retry) via dispatch-failure eviction
- Zero overhead on the warm dispatch path — no extra Podman calls or health checks
- `engine_version` failure at startup degrades gracefully to `"unknown"`
- Both modes populate the same `ContainerRuntimeInfo` struct — diagnostics are consistent regardless of policy

### Deferred

1. **Configurable health-check policy** — `runtime.health_check` setting with modes: `on-failure` (default), `before-dispatch`, `none`. Trades latency for richer guarantees. Gets its own ADR when needed.

2. **Agent-side attestation** — When the agent contract evolves to structured JSON responses, the agent can report its own view (SDK version, process uptime). The platform validates from the outside (Podman metadata), the agent attests from the inside. Two complementary views, both part of the protocol.
