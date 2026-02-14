# ADR 084: Container Identity

## Status

Draft

## Context

Running agent containers have no platform-issued identity. Today, the platform knows about containers through infrastructure artifacts — a Podman container ID, a host port, an agent name. None of these are a credential the container can present to prove who it is.

This absence shows up in several places:

**Git commit authorship.** The conversation store records every side effect as a git commit. The author is `todoapp@127.0.0.1:9000` — the agent name concatenated with the registry host. This is a fabricated string that encodes infrastructure details (which registry, which port) rather than a real identity. It cannot distinguish between two instances of the same agent. It doesn't survive registry moves.

**Inference proxy identification.** The inference passthrough ADR introduces per-provider proxy ports on the bridge. A single always-on proxy is simpler than per-container lifecycle management — but a single proxy serving multiple agents needs to know which agent is making each request. Standard SDKs (OpenAI, LangChain) send an API key as `Authorization: Bearer <token>` on every request. This is the natural injection point, but there is nothing to inject — containers have no token.

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

### Applicability to Vlinder

Vlinder controls the full container lifecycle — `podman pod create`, `podman run`, secret injection. The platform knows what it's deploying and can generate credentials without any participation from the agent author. The SPIFFE JWT-SVID format provides an industry-standard claim structure and a clean upgrade path to full SPIRE if attestation or automatic rotation is ever needed.

## Decision

### Platform generates the key pair

The platform generates an asymmetric key pair when an agent is first deployed. Deploy is idempotent — subsequent deploys reuse the existing key pair. The agent author never touches cryptography.

The private key is stored as a Podman secret. The public key is stored in the registry alongside the agent's `ResourceId`. No `[identity]` section in `agent.toml`. No key generation for authors. Identity is infrastructure, managed by the platform.

```bash
# Platform does this internally on first deploy:
podman secret create todoapp-key /path/to/generated/private.pem
```

### Instance identity

Multiple containers of the same agent share the same key pair — identity is per-agent, not per-instance. To distinguish instances, the platform assigns a unique instance ID when starting each container and passes it to the sidecar:

```bash
podman run --pod todoapp-pod --secret todoapp-key -e VLINDER_INSTANCE_ID=abc123 -d vlinder-sidecar
```

The sidecar includes the instance ID in the JWT claims:

```json
{
  "sub": "spiffe://vlinder.local/agents/todoapp",
  "instance": "abc123",
  "aud": ["vlinder-proxy"],
  "iat": 1707951600,
  "exp": 1707955200
}
```

Same key pair, distinct instances. The DAG can attribute each side effect to a specific container.

### Sidecar container in a Podman pod

The platform runs agent containers inside a Podman pod alongside a platform-owned sidecar. The sidecar handles all credential presentation — the agent container has no Vlinder infrastructure baked in.

```
podman pod create --name todoapp-pod -p 8080:8080
podman run --pod todoapp-pod --secret todoapp-key -d vlinder-sidecar
podman run --pod todoapp-pod -d todoapp-agent
```

The sidecar receives the private key via Podman secrets at `/run/secrets/todoapp-key`. The agent container has no access to the secret — only the sidecar does. Even a compromised agent cannot exfiltrate the key.

All containers in a pod share the same network namespace. The agent reaches the sidecar at `localhost`. The sidecar reaches the platform at the host address. No special networking configuration.

The sidecar is a platform-owned OCI image, versioned and updated independently of agent images. Agent images stay pure — just agent code and dependencies.

### Sidecar intercepts and signs

The agent makes standard HTTP calls using standard SDKs (OpenAI, LangChain, LiteLLM). The `VLINDER_OPENROUTER_URL` env var points to the sidecar inside the pod, not directly to the platform proxy:

```
VLINDER_OPENROUTER_URL=http://localhost:19999/openrouter
```

Agent code is unmodified:

```python
client = OpenAI(
    base_url=os.environ["VLINDER_OPENROUTER_URL"] + "/v1",
    api_key="unused",
)
```

On each request, the sidecar:

1. Receives the HTTP request from the SDK on localhost
2. Reads the private key from `/run/secrets/todoapp-key`
3. Signs a SPIFFE-compatible JWT-SVID: `{ sub: "spiffe://vlinder.local/agents/todoapp", aud: ["vlinder-proxy"], iat, exp }` with the private key
4. Adds `X-Vlinder-Identity: <signed-jwt>` header (leaves `Authorization` untouched for provider auth)
5. Forwards to the platform proxy
6. Returns the response unmodified

The agent never participates in identity. The SDK never knows. Standard libraries, standard code, identity is invisible infrastructure — the same pattern as Istio/Envoy service mesh sidecars, without Kubernetes.

### Platform verifies with the public key

The platform proxy (inference passthrough ADR) receives each request:

1. Reads the JWT from `X-Vlinder-Identity`
2. Extracts `agent_id` from the claims
3. Looks up the registered public key for that agent
4. Verifies the JWT signature
5. Proceeds with the verified identity: model validation, NATS routing, DAG recording

Verification is local — no callback to an identity service. The public key is loaded from the registry where it was stored at first deploy.

### What this replaces

The fabricated author email (`todoapp@127.0.0.1:9000`) is replaced by a verified `agent_id` derived from the JWT claims. Today there is no authentication between containers and the platform — the sidecar adds it. Multi-instance disambiguation comes from the `instance` claim in the JWT, assigned by the platform at container start.

## Consequences

- Zero burden on agent authors — the platform generates and manages credentials, no `[identity]` config
- Agent images contain no Vlinder infrastructure — the sidecar is platform-owned
- Standard SDKs work unmodified — identity injection is invisible to agent code
- Podman pods provide shared-localhost networking between agent and sidecar
- Podman secrets isolate the private key to the sidecar — the agent container cannot access it
- The sidecar is versioned independently — upgrades don't require rebuilding agent images
- Asymmetric signing means verifiers only need the public key — no shared secrets across distributed workers
- SPIFFE-compatible JWT-SVID format — upgrade path to full SPIRE if needed, interoperable with SPIFFE-aware services
- Git commit authorship uses verified identity, not fabricated strings
- The inference proxy identifies callers through verified JWTs, enabling a single always-on proxy for all agents
- Container runtime changes from `podman run` to `podman pod create` + `podman run --pod` — the pod becomes the unit of deployment
- Service worker identity (inference, storage, DAG workers authenticating to NATS) is a separate problem — future ADR
