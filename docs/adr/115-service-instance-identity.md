# ADR 115: Service Instance Identity

**Status:** Draft

## Context

Services are identified by type. `ServiceBackend::Kv(Sqlite)` renders
as `"kv.sqlite"` on the wire and in `DagNode.to`. The manifest enforces
one backend per service type:

```rust
pub services: HashMap<ServiceType, ServiceConfig>,
```

```toml
[requirements.services.kv]
provider = "sqlite"
```

There is no concept of a service *instance* — only a service *type*.

### Where this breaks

1. **Multiple stores** — an agent that needs two KV stores (e.g., todos
   and settings) can't declare them. The manifest has one slot per
   service type.

2. **Ambiguous routing** — `DagNode.to = "kv.sqlite"` doesn't
   distinguish which SQLite KV store a request is for. The provider
   can't route to the right worker.

3. **State scoping** — the state trailer on a DagNode is a composite
   snapshot of all KV state. Deriving the state of a *specific* KV
   store on a *specific* timeline requires knowing which KV store a
   response came from. `"kv.sqlite"` doesn't tell you that.

### What currently exists

- `ServiceBackend` is an enum: `Kv(ObjectStorageType)`,
  `Vec(VectorStorageType)`, `Infer(InferenceBackendType)`,
  `Embed(EmbeddingBackendType)`. Type + backend, no instance.

- Wire format is two segments: `"kv.sqlite"`, `"infer.openrouter"`.

- `object_storage: Option<ResourceId>` and
  `vector_storage: Option<ResourceId>` on `AgentManifest` are
  single-valued fields, separate from the services map.

- `RoutingKey::Request` and `RoutingKey::Response` carry
  `ServiceBackend`. Adding instance identity there propagates
  through routing automatically.

## Decision

### Ownership

Service instances are agent-scoped. Each instance is owned by exactly
one agent. No sharing across agents — if two agents need the same data,
they communicate through the protocol.

### Identity

A service instance is identified by a user-chosen name in the manifest,
scoped to the agent. The name is the primary key. Type and provider are
properties of the instance, derivable from the URI.

Uniqueness is enforced per agent — two instances with the same name
in the same agent manifest is a validation error.

### Manifest syntax

Flat map of named services. Name is the key, URI is the connection
string. The platform derives type and provider from the URI scheme.

```toml
[services.orders]
type = "kv"
uri = "dynamodb://us-east-1/orders-table"

[services.inventory]
type = "kv"
uri = "dynamodb://us-west-2/inventory-table"

[services.reasoning]
type = "infer"
uri = "openrouter://anthropic/claude-3.5-sonnet"
protocol = "openai"

[services.embeddings]
type = "embed"
uri = "ollama://nomic-embed-text"
```

Type is always explicit — some URI schemes are ambiguous (ollama does
both infer and embed, sqlite does both kv and vec).

This follows Docker Compose and Kubernetes conventions — instances are
named, types are properties, not grouping keys.

### Agent access

Agents access services via the existing `*.vlinder.local` convention.
The instance name becomes the hostname:

```
orders.vlinder.local:3544/get?key=foo
inventory.vlinder.local:3544/get?key=foo
reasoning.vlinder.local:3544/v1/chat/completions
```

The sidecar resolves `*.vlinder.local` and maps instance names to
the correct backend, same as it maps backend types today.

### Wire identity

`DagNode.to` and `DagNode.from` carry the instance name:
`"orders"`, `"inventory"`, `"reasoning"`. The instance name is the
address on the wire.

### ServiceBackend

`ServiceBackend` remains the catalog of available service types and
backends (e.g., `Kv(Sqlite)`, `Infer(OpenRouter)`). It does not gain
an instance name. The instance name is a separate concept — it's the
user-chosen identity that points to a `ServiceBackend`.

### Backend-specific config

Everything beyond `uri` in a service declaration is backend-specific
config. The manifest passes it through without interpreting it.
Different backends have different config shapes:

```toml
[services.reasoning]
uri = "openrouter://anthropic/claude-3.5-sonnet"
protocol = "openai"

[services.orders]
uri = "postgres://localhost/orders"
user = "app"
password_secret = "orders-db-pass"
```

`protocol` is not a top-level manifest concept — it's inference-specific
config (which API flavor the agent speaks). For other backends, the
config fields will be different.
