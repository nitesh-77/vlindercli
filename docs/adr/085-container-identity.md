# ADR 085: Container Identity

## Status

Draft

## Context

Running agent containers have no platform-issued identity. Today, the platform knows about containers through infrastructure artifacts — a Podman container ID, a host port, an agent name. None of these are a credential the container can present to prove who it is.

This absence shows up in several places:

**Git commit authorship.** The conversation store records every side effect as a git commit. The author is `todoapp@127.0.0.1:9000` — the agent name concatenated with the registry host. This is a fabricated string that encodes infrastructure details (which registry, which port) rather than a real identity. It cannot distinguish between two instances of the same agent. It doesn't survive registry moves.

**Inference proxy identification.** ADR 083 introduces per-provider proxy ports on the bridge. A single always-on proxy is simpler than per-container lifecycle management — but a single proxy serving multiple agents needs to know which agent is making each request. Standard SDKs (OpenAI, LangChain) send an API key as `Authorization: Bearer <token>` on every request. This is the natural injection point, but there is nothing to inject — containers have no token.

**Multi-instance ambiguity.** The architecture supports multiple containers of the same agent (agents are stateless). But today there is no way to distinguish which instance produced a given side effect. The DAG records the agent name, not the instance.

**Implicit trust.** The platform trusts that any request arriving on a container's bridge is from that container. There is no authentication — the binding is positional (this port belongs to this agent) rather than cryptographic. This works for localhost single-tenant but breaks for any distributed or multi-tenant scenario.

The common root: containers are identified by where they are (port, process), not by who they are (credential). A platform-issued identity — a token generated at container start and injected as an environment variable — would give containers a portable, verifiable credential they can present on any interaction with the platform.

## Considerations

### Industry precedent: SPIFFE/SPIRE

The [SPIFFE](https://spiffe.io/) (Secure Production Identity Framework for Everyone) standard defines how workloads identify themselves to each other. [SPIRE](https://spiffe.io/docs/latest/spire-about/) is its production implementation, deployed at Uber, Square, Bloomberg, and across the CNCF ecosystem.

The architecture:

- **SPIRE Server** — the certificate authority. Issues short-lived identity documents (SVIDs) to attested workloads.
- **SPIRE Agent** — runs on each node. Performs workload attestation (proving a process is what it claims to be) using platform-specific evidence: Kubernetes pod metadata, AWS instance identity documents, Docker container labels, Unix process attributes.
- **Attestation** — the agent proves the workload's identity to the server using evidence the workload cannot forge: kernel PIDs, container IDs from the runtime API, cloud provider signed metadata.
- **SVID** — the issued credential. Either an X.509 certificate (for mTLS) or a JWT (for bearer auth). Short-lived (minutes to hours), automatically rotated, cryptographically verifiable.

The key insight: identity is *attested*, not *self-declared*. The workload doesn't choose its name — the platform verifies what it is and issues a credential asserting that identity.

### Kubernetes service accounts

Kubernetes projects a signed JWT into every pod via a mounted volume (`/var/run/secrets/kubernetes.io/serviceaccount/token`). The token is bound to a ServiceAccount and namespace. API server validates it. Since 1.22, tokens are audience-bound and time-limited (projected service account tokens).

This is simpler than SPIFFE — no separate attestation step. The platform controls the container lifecycle and injects the credential during scheduling. The container can present the token to any service that trusts the API server as an issuer.

### Cloud provider managed identity

AWS IAM Roles for Tasks (ECS), GCP Workload Identity, Azure Managed Identity — all follow the same pattern: the platform assigns an identity to the workload at scheduling time and makes credentials available via a metadata endpoint or injected token. The workload never holds long-lived secrets.

### What this means for Vlinder

Vlinder controls the full container lifecycle — `podman run` with known parameters. This is analogous to Kubernetes scheduling a pod: the platform knows exactly what it's starting, what agent it belongs to, and what session it's part of. Full SPIFFE/SPIRE is overkill for a single-node, single-tenant system where the platform *is* the scheduler. But the core pattern applies directly:

1. **Platform generates credential at container start** — a token encoding agent identity, instance ID, session binding.
2. **Injected via environment variable** — the container receives it without any registration or handshake.
3. **Presented on every platform interaction** — `Authorization: Bearer <token>` on inference proxy requests, included in DAG metadata for git authorship.
4. **Verified by the platform** — the proxy, the bridge, the DAG writer all validate the token against what the platform issued.

The question is what the token contains and how it's verified — not whether containers need identity.
