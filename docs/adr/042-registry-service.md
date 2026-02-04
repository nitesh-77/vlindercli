# ADR 042: Registry Service

## Status

Proposed

## Context

The `Registry` trait is the source of truth for agents, models, jobs, and capabilities. Currently it's in-process only (`InMemoryRegistry`). For distributed mode (ADR 043), workers need network access to the registry.

### Options Considered

1. **NATS request/reply** — Use existing queue infrastructure
2. **HTTP/REST** — Simple, curl-friendly
3. **gRPC** — Efficient, streaming support

## Decision

**gRPC for the Registry service.**

### Why gRPC

1. **Typed IPC** — These services exist in the same codebase, for each other. Protobuf extends type safety across process boundaries. Contract changes break compilation, not production.

2. **Same trait, different transports** — `InMemoryRegistry` for tests, `GrpcRegistryClient` for distributed. Same interface, no network in tests.

3. **Future optionality** — Streaming available if needed. Bonus, not driver.

### Service Definition

```protobuf
service Registry {
  rpc GetAgent(GetAgentRequest) returns (Agent);
  rpc GetModel(GetModelRequest) returns (Model);
  rpc RegisterAgent(Agent) returns (RegisterResponse);
  rpc RegisterModel(Model) returns (RegisterResponse);
  rpc CreateJob(CreateJobRequest) returns (Job);
  rpc GetJob(GetJobRequest) returns (Job);
  rpc UpdateJobStatus(UpdateJobRequest) returns (UpdateJobResponse);
  rpc ListPendingJobs(Empty) returns (JobList);
}
```

### Architecture

```
      ┌──────────┐
      │  Harness │  ← User-facing API
      └────┬─────┘
           │
    ┌──────┴──────┐
    ▼             ▼
┌──────────┐  ┌──────────┐
│ Registry │  │   NATS   │
│ (gRPC)   │  │ (Queue)  │
└────┬─────┘  └────┬─────┘
     │             │
     └──────┬──────┘
            ▼
      ┌──────────┐
      │  Workers │
      └──────────┘
```

- **Harness**: User-facing, simplified API, auth boundary
- **Registry**: Internal gRPC service, service-to-service
- **Workers**: Query registry for config, receive work from queue

### Client Implementation

```rust
pub struct GrpcRegistryClient {
    client: RegistryClient<Channel>,
}

impl Registry for GrpcRegistryClient {
    fn get_agent(&self, id: &ResourceId) -> Option<Agent> {
        // gRPC call
    }
    // ... implements full Registry trait
}
```

Workers use `GrpcRegistryClient`. Same `Registry` trait, different transport.

## Consequences

**Positive:**
- Workers can query registry from any process/machine
- Compile-time contract verification
- Same trait for tests (in-memory) and production (gRPC)
- Streaming available for future health checks if needed

**Negative:**
- Adds tonic/prost dependency
- Requires protobuf compilation step
- Another service to run in distributed mode

## Implementation Phases

1. Add tonic/prost dependencies, define .proto
2. Implement gRPC server wrapping existing Registry
3. Implement `GrpcRegistryClient` (implements `Registry` trait)
4. Workers use `GrpcRegistryClient` in distributed mode
