# ADR 109: Lambda Runtime

**Status:** Accepted

## Context

The container runtime runs agents as local Podman containers with a sidecar
for provider services. This works for single-machine deployment but doesn't
scale — each agent occupies a long-running container on the host.

Lambda offers serverless execution: no provisioned containers, pay-per-invoke,
and automatic scaling. But Lambda has constraints the container runtime
doesn't: read-only root filesystem, no sidecar process model, and the agent
must connect back to platform services (NATS, registry, state) over the
network rather than localhost.

The question: can the existing agent contract (HTTP health + invoke on port
8080) and provider server (virtual-host routing) work inside Lambda without
changing the agent code?

## Decision

### Three components

1. **LambdaRuntime** (daemon worker) — owns the full Lambda function
   lifecycle. Each tick: query registry for Lambda agents, remove orphan
   functions, deploy missing agents, dispatch invocations from NATS.

2. **vlinder-lambda-adapter** (Lambda extension binary) — replaces the
   container sidecar. Installed at `/opt/extensions/` in the agent image.
   Registers with the Extensions API, connects to NATS/registry/state,
   runs a ProviderServer, and handles the Runtime API loop.

3. **Agent image** — same HTTP contract as container agents. Includes the
   adapter binary and a baked-in `/etc/hosts` for `.vlinder.local` DNS.

### Function lifecycle is daemon-managed

`LambdaRuntime` creates and destroys all AWS resources directly:

- **Deploy**: create IAM role `vlinder-agent-{name}` (trust policy + KMS
  permissions), wait 10s for IAM propagation, create Lambda function
  `vlinder-{name}` from ECR image with VPC config and `VLINDER_*` env vars.
- **Reconcile**: `ensure_functions()` mirrors `ContainerRuntime`'s pool —
  orphan removal + missing deployment, driven by registry state.
- **Dispatch**: poll NATS invoke queue, serialize `InvokeMessage` as Lambda
  payload, call `invoke_function`. On Lambda-level failure, daemon sends
  error `CompleteMessage` so the harness doesn't hang.
- **Shutdown/Drop**: undeploy all functions (delete Lambda + IAM role).
  All operations are idempotent.

### Adapter handles the invocation

Inside Lambda, the adapter replaces the sidecar:

1. Register with Extensions API (must complete within init timeout).
2. Connect to NATS, registry, state using `VLINDER_*` env vars.
3. Wait for agent health (up to 60s).
4. Runtime API loop: GET next invocation → deserialize `InvokeMessage` →
   start `ProviderServer` → POST to agent `/invoke` → send
   `CompleteMessage` to NATS → POST response to Runtime API.

The adapter sends complete directly to NATS — the daemon only sees
invoke-level failures.

### ProviderServer shared between runtimes

`vlinder-provider-server` crate is used by both the container sidecar and
the Lambda adapter. Two fixes were needed for Lambda:

- **Port-qualified hostnames**: `start()` appends `:{port}` to each
  hostname so `match_route` works against Host headers (which include port).
- **Baked-in /etc/hosts**: Lambda's root FS is read-only. DNS for
  `.vlinder.local` is COPY'd into the image at build time.

### Build targets AL2023

CI builds ARM binaries inside an Amazon Linux 2023 container (glibc 2.34)
to match both Lambda and EC2 deployment targets.

## Consequences

- `runtime = "lambda"` in agent.toml is all that's needed — the daemon
  handles IAM, Lambda functions, and invocation dispatch
- Agent code is identical between container and Lambda runtimes
- Agent images must include the adapter binary and baked-in `/etc/hosts`
- ARM binaries must be built in AL2023 for glibc compatibility
