# ADR 083: Secret Store

## Status

Draft

## Context

ADR 084 (Container Identity) generates Ed25519 private keys for agent containers. ADR 085 (Service Identity) generates NKeys for NATS service workers. Both need somewhere to store secrets that is:

1. **Accessible from all components.** In distributed mode, the component that generates a secret (harness, daemon) may not be the component that consumes it (runtime worker on a different machine). Secrets must be reachable from any worker.

2. **Encrypted at rest.** Private keys in plaintext SQLite is not acceptable. The registry database is not a secret store.

3. **Persistent across restarts.** Agent identity is generated once at first deploy. The private key must survive daemon restarts without regeneration.

4. **Not machine-local.** Podman secrets are stored in the local Podman instance. A runtime worker on machine B cannot read a Podman secret created on machine A. Machine-local storage breaks distributed deployments.

Today, the platform has no secret storage at all. API keys (OpenRouter) live in config files. Agent private keys don't exist yet. NATS NKeys don't exist yet. As identity lands (ADR 084, 085), the platform needs a first-class secret store.

### What needs to be stored

| Secret | Producer | Consumer |
|--------|----------|----------|
| Agent Ed25519 private key (ADR 084) | Runtime at first container start | Runtime at subsequent container starts |
| NATS NKeys (ADR 085) | Daemon/Supervisor at startup | Service workers at startup |
| Provider API keys (OpenRouter) | User configuration | Inference workers |

## Decision

### SecretStore trait

A new domain trait with the same pattern as `MessageQueue`, `Registry`, and `RegistryRepository` — trait defines the contract, implementations are pluggable.

```rust
pub trait SecretStore: Send + Sync {
    fn put(&self, name: &str, value: &[u8]) -> Result<(), SecretStoreError>;
    fn get(&self, name: &str) -> Result<Vec<u8>, SecretStoreError>;
    fn exists(&self, name: &str) -> Result<bool, SecretStoreError>;
    fn delete(&self, name: &str) -> Result<(), SecretStoreError>;
}
```

Secrets are named byte blobs. The store does not interpret contents — it stores and retrieves. Naming convention: `agents/{name}/private-key`, `nats/{role}/nkey`, `providers/{name}/api-key`.

### NATS KV implementation

NATS is already in the stack. Every component connects to NATS. JetStream provides a KV store with at-rest encryption — no new dependency.

```rust
pub struct NatsSecretStore {
    kv: nats::jetstream::kv::Store,  // bucket: "vlinder-secrets"
}
```

The NATS KV bucket `vlinder-secrets` is created at daemon startup with:
- Replicas matching the NATS cluster size
- History depth of 1 (latest value only — no secret history)
- Max value size capped (private keys are small)

Subject-level access control (ADR 085) restricts which components can read which secrets. The secret store bucket subjects are included in the NKey account permissions.

### InMemory implementation for tests

```rust
pub struct InMemorySecretStore {
    secrets: Mutex<HashMap<String, Vec<u8>>>,
}
```

Same pattern as `InMemoryQueue` and `InMemoryRegistry` — unit tests never touch real infrastructure.

### Integration with ADR 084 (Container Identity)

At first container start, the runtime:
1. Checks `secret_store.exists("agents/{name}/private-key")`
2. If missing: generates Ed25519 key pair, calls `secret_store.put(...)`, updates `public_key` in registry
3. Reads private key from secret store
4. Creates a Podman secret from the retrieved key (local to this machine, ephemeral)
5. Starts the pod with `--secret` flag

The Podman secret is a local cache of the canonical secret. If the Podman secret is missing but the secret store has the key, the runtime recreates it. The secret store is the source of truth.

### Integration with ADR 085 (Service Identity)

At daemon startup:
1. Generate NKeys for each worker role (if not already in secret store)
2. Store private NKeys in secret store
3. Pass credentials to workers at spawn time

## Consequences

- Private keys never touch SQLite or gRPC — the secret store is the only persistence layer for secrets
- NATS KV provides encryption at rest with zero additional dependencies
- All components can access secrets through the same NATS connection they already use
- Podman secrets become a local cache, not the source of truth — runtime workers on any machine can retrieve agent keys
- The `SecretStore` trait is testable with `InMemorySecretStore` — no infrastructure needed for unit tests
- Secret naming convention (`agents/*/private-key`, `nats/*/nkey`) provides natural access control boundaries
- Provider API keys (OpenRouter) can migrate from config files to the secret store in a future iteration
