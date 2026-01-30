# ADR 032: vlinderd as Runtime Registry

## Status

Proposed

## Context

ADR 018 established queue-based message passing. ADR 031 defined harnesses as user-facing interfaces.

The current `CliHarness` embeds `WasmRuntime` directly — it creates and manages the runtime in-process. This coupling was useful for learning (see ADR 031) but now limits us: harnesses should work with any runtime, local or remote.

## Decision

**vlinderd is the central registry and runtime manager.**

### Harness is a Thin Client

Harnesses (CLI, Web, API) do NOT create runtimes. They:

1. Ask vlinderd for discovery (agents, models, runtimes)
2. Get runtime connection info from vlinderd
3. Communicate with runtimes via the provided interface

```
CliHarness
    │
    ├─► vlinderd: "resolve agent 'pensieve'"
    │   └─► { runtime: "wasm-local", endpoint: "unix:///var/run/vlinder/wasm.sock" }
    │
    └─► Runtime (via socket)
        └─► executes agent, returns response
```

### vlinderd Manages Runtime Lifecycle

vlinderd boots, monitors, and provides endpoints for all runtimes:

```
vlinderd
    │
    ├── boots WasmRuntime (local process, unix socket)
    ├── connects to AwsRuntime (Lambda API)
    ├── connects to K8sRuntime (K8s API)
    │
    └── exposes discovery API
        ├── GET /runtimes → list available runtimes
        ├── GET /agents/{name} → { runtime, endpoint, status }
        └── POST /agents/{name}/invoke → proxied or direct?
```

### Runtime Interface

All runtimes expose the same interface, transport varies:

| Runtime | Transport | Endpoint Example |
|---------|-----------|------------------|
| WasmRuntime | Unix socket | `unix:///var/run/vlinder/wasm.sock` |
| WasmRuntime | TCP | `tcp://localhost:9000` |
| AwsRuntime | HTTPS | `https://lambda.us-east-1.amazonaws.com` |
| K8sRuntime | HTTPS | `https://k8s.cluster.local` |

The harness doesn't care — it gets an endpoint from vlinderd and talks to it.

### Runtime Trait

```rust
pub trait Runtime: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> RuntimeCapabilities;

    // Agent lifecycle
    fn create(&self, agent: &Agent) -> Result<(), RuntimeError>;
    fn delete(&self, agent_name: &str) -> Result<(), RuntimeError>;
    fn status(&self, agent_name: &str) -> Result<AgentStatus, RuntimeError>;
}

pub struct RuntimeCapabilities {
    pub executor: ExecutorType,  // wasm, lambda, container
    pub models: Vec<String>,     // available models
    pub gpu: bool,
}
```

### Discovery Flow

```
1. User: vlinder agent run -p ./my-agent

2. CLI asks vlinderd:
   → POST /agents/my-agent/resolve
   ← { runtime: "wasm-local", endpoint: "unix:///var/run/vlinder/wasm.sock" }

3. CLI sends request to runtime endpoint:
   → Runtime executes agent
   ← Response

4. CLI displays result
```

### What vlinderd Tracks

```
vlinderd
    │
    ├── Runtimes
    │   ├── wasm-local (spawned by vlinderd, unix socket)
    │   ├── aws-prod (configured, HTTPS to Lambda)
    │   └── k8s-dev (configured, HTTPS to cluster)
    │
    ├── Models
    │   └── phi3, nomic-embed, ... (paths, capabilities)
    │
    ├── Agents
    │   └── pensieve → { runtime: wasm-local, status: running }
    │
    └── Fleets
        └── my-fleet → { agents: [...], status: deployed }
```

## Consequences

- **Harness is decoupled**: No embedded runtime, just a thin client
- **Uniform interface**: Same harness code works with any runtime
- **vlinderd is the authority**: Single source of truth for what's running where
- **Runtimes are processes**: Even local WasmRuntime is a separate process managed by vlinderd
- **Testability**: Can mock vlinderd responses for testing harnesses

## Migration

Current `CliHarness` embeds `WasmRuntime`. Migration path:

1. Extract WasmRuntime to separate process with socket interface
2. Add vlinderd with runtime registry
3. Update CliHarness to query vlinderd instead of embedding runtime
4. Existing tests use `TestHarness` that embeds everything (no vlinderd needed)

## Open Questions

- Protocol for harness ↔ runtime communication (gRPC? HTTP? custom?)
- vlinderd API design (REST? gRPC?)
- Runtime health checks and automatic restart
- Local development mode (vlinderd + WasmRuntime in one process?)
