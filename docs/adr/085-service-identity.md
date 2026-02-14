# ADR 085: Service Identity

## Status

Draft

## Context

ADR 084 solves container identity — agent containers authenticate to the platform via a sidecar-signed JWT. But the platform's own service workers have no identity either.

Service workers — inference, embedding, storage, state, DAG — are processes that connect to NATS and subscribe to subjects. In distributed mode, they run as separate processes, potentially on separate machines. Today, there is no authentication at any layer.

This shows up in several places:

**NATS is open.** Any process that can reach the NATS server can subscribe to any subject. A rogue process subscribing to `vlinder.*.req.*.infer.openrouter.*` receives inference requests intended for the real worker — including full prompt content. There is no mechanism to restrict which processes handle which subjects.

**No response verification.** When the bridge sends a request through NATS and receives a response, it trusts the response unconditionally. There is no verification that the response came from a legitimate worker rather than a rogue subscriber.

The common root: NATS subjects are the only boundary between service workers, and they are not access-controlled.

Note: gRPC endpoint authentication (registry, state service) is a separate concern — future ADR.

## Decision

### NATS built-in auth with NKey accounts

NATS provides a JWT account system with subject-level publish/subscribe permissions. The platform generates NKeys (Ed25519 key pairs) for each worker role and configures NATS to enforce subject permissions.

Each worker role gets an account with the minimum subjects it needs:

```
inference-openrouter:
  subscribe: vlinder.*.req.*.infer.openrouter.>
  publish:   vlinder.*.resp.>

inference-ollama:
  subscribe: vlinder.*.req.*.infer.ollama.>
  publish:   vlinder.*.resp.>

embedding-ollama:
  subscribe: vlinder.*.req.*.embed.ollama.>
  publish:   vlinder.*.resp.>

storage-sqlite:
  subscribe: vlinder.*.req.*.kv.sqlite.>
  publish:   vlinder.*.resp.>

storage-memory:
  subscribe: vlinder.*.req.*.kv.memory.>
  publish:   vlinder.*.resp.>

vector-sqlite:
  subscribe: vlinder.*.req.*.vector.sqlite-vec.>
  publish:   vlinder.*.resp.>

vector-memory:
  subscribe: vlinder.*.req.*.vector.memory.>
  publish:   vlinder.*.resp.>

dag-git:
  subscribe: vlinder.>
  publish:   (none)

bridge:
  publish:   vlinder.*.req.>
  subscribe: vlinder.*.resp.>
```

A compromised inference worker cannot read storage requests. A compromised storage worker cannot read inference prompts. Subject-level isolation, enforced by NATS itself.

### Platform generates and distributes credentials

The daemon generates all NKeys and NATS account JWTs at startup. Workers receive their credentials as arguments or environment variables when the daemon spawns them. No worker manages its own auth — the platform owns everything.

This is the same principle as ADR 084: the platform controls the lifecycle, the platform assigns the credential. Workers are platform-owned processes — there is no reason to burden them with identity management.

### No application-level changes

Workers connect to NATS with credentials. The `MessageQueue` trait and `NatsQueue` implementation handle auth at the connection layer. Worker code (`InferenceServiceWorker`, `ObjectServiceWorker`, etc.) is unchanged — they call `receive_request()` and `send_response()` as before. Subject permission enforcement is transparent.

## Consequences

- Subject-level isolation between worker roles — inference can't see storage, storage can't see inference
- NATS enforces permissions at the server — no application-level auth code needed
- Credentials are generated and distributed by the daemon — zero burden on operators
- NKeys use Ed25519 — same cryptographic primitive as ADR 084 container identity
- Rogue NATS subscribers are rejected — connection requires a valid NKey for an authorized account
- Worker code is unchanged — auth is at the connection layer, not the application layer
- gRPC endpoint authentication (registry, state service) is out of scope — separate ADR
