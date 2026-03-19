# ADR 120: Provider Plugin Contract

**Status:** Draft

## Context

The sidecar is the agent's single point of contact with the outside world. It proxies service calls, captures interactions, and emits messages on the queue. Today the sidecar has hardcoded knowledge of its provider crates — it knows `vlinder-ollama` is HTTP, it knows how to start the `tiny_http` provider server, it knows which routes to register.

ADR 118 introduces HTTP wire-format capture for stateless services. ADR 119 introduces a Postgres wire protocol adapter for Dolt. These are fundamentally different: HTTP is a stateless request-response proxy, Postgres is a stateful connection-per-session proxy with control plane injection.

The sidecar should not encode which protocol a provider uses. Each provider crate should bring its own protocol listener, connection lifecycle, message emission, and state hash extraction. The sidecar just runs whatever the agent's manifest declares.

## Decision

### 1. Provider plugin trait

A provider crate implements a plugin trait that the sidecar calls to start the provider. The trait is the contract between the sidecar and any provider — HTTP, Postgres, or future protocols.

The trait provides:
- A way to start listening (HTTP server, TCP listener, etc.)
- Access to the message queue for emitting request/response messages
- Access to session context (branch, submission) for control plane operations
- A way to report state hashes for the DAG node snapshot

The sidecar does not know the protocol. It starts the plugin and lets it run.

### 2. Each provider crate owns its full lifecycle

- `vlinder-ollama`: starts an HTTP listener, registers routes, proxies to Ollama, captures full HTTP responses, emits messages
- `vlinder-infer-openrouter`: same pattern, different upstream
- `vlinder-dolt`: starts a Postgres listener, manages connection-per-session, injects Dolt control plane, captures wire bytes, extracts commit hash, emits messages
- Future providers: implement the trait with whatever protocol they need

### 3. Agent manifest drives plugin selection

The agent's manifest declares its requirements. The sidecar reads the manifest and starts the corresponding plugins. If the agent declares `database = "postgres://..."`, the sidecar starts the Dolt plugin. If it declares inference models, the sidecar starts the inference plugin.

No provider runs unless the agent needs it.

## Consequences

- The sidecar becomes a plugin host, not a protocol-aware proxy
- Adding a new provider (Redis, S3, DynamoDB) requires a new crate implementing the trait — no sidecar changes
- Each provider crate is self-contained: protocol, lifecycle, message emission, state extraction
- The HTTP provider host (`ProviderHost`/`ProviderRoute`) becomes one implementation of the trait, not a privileged abstraction
- The trait must be general enough for stateless (HTTP) and stateful (Postgres) providers without forcing either model on the other
