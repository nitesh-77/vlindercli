# ADR 024: Executor-Aware Manifest Format

## Status

Proposed

## Context

The current agent manifest format implicitly assumes WASM execution:

```toml
name = "my-agent"
description = "Does something useful"
code = "agent.wasm"

[requirements.models]
phi3 = "file://./models/phi3.toml"

[[mounts]]
host_path = "mnt"
guest_path = "/"
mode = "rw"
```

Problems with this:

1. **`[[mounts]]` is WASI-specific** — only makes sense for WASM executors with filesystem sandbox. Lambda, containers, and remote agents have entirely different storage models.

2. **`code` interpretation is implicit** — we infer "WASM" from `.wasm` extension. A Lambda agent would need `function_arn`, a container needs `image`.

3. **No validation** — you could add `[[mounts]]` to what's intended as a Lambda agent. The code would parse it, the executor would ignore it.

4. **Agent struct has WASM fields at domain level** — `agent_dir`, `mounts`, `db_path()` assume local filesystem execution.

The "write once, run anywhere" agent is aspirational. In reality, executors have fundamentally different capabilities:

| Capability | WASM | Lambda | Container |
|------------|------|--------|-----------|
| Filesystem mounts | WASI preopened dirs | No (use S3) | Docker volumes |
| Storage | Local SQLite | DynamoDB/S3 | Varies |
| Networking | Host-controlled | VPC config | Container network |
| Code artifact | `.wasm` file | Function ARN | Container image |

## Decision

**Make executor explicit in manifest. Executor-specific config lives in dedicated sections.**

### New format

```toml
name = "my-agent"
description = "Does something useful"
executor = "wasm"

[requirements.models]
phi3 = "file://./models/phi3.toml"
nomic = "file://./models/nomic.toml"

[wasm]
module = "agent.wasm"

[[wasm.mounts]]
host_path = "mnt"
guest_path = "/"
mode = "rw"
```

For Lambda (future):

```toml
name = "my-agent"
description = "Does something useful"
executor = "lambda"

[requirements.models]
claude = "bedrock://claude-3-sonnet"

[lambda]
function_arn = "arn:aws:lambda:us-east-1:123456789:function:my-agent"
region = "us-east-1"
```

Note: Model URIs point to model manifests (per ADR 023), which declare the model type, engine, and actual file location.

### Schema

**Top-level (all executors):**
- `name` (required) — agent identity
- `description` (required) — human-readable description
- `executor` (required) — one of: `wasm`, `lambda`, `container`
- `source` (optional) — source repository URL
- `[requirements]` — models and services needed
- `[prompts]` — prompt template overrides

**`[wasm]` section:**
- `module` (required) — path to `.wasm` file, relative to agent dir
- `[[wasm.mounts]]` — WASI filesystem mounts

**`[lambda]` section (future):**
- `function_arn` (required)
- `region` (required)

**`[container]` section (future):**
- `image` (required)
- `[[container.volumes]]` — volume mounts

### Validation

- Parser rejects unknown executor values
- Parser rejects executor-specific sections that don't match declared executor
- `[[mounts]]` at top level is an error (must be `[[wasm.mounts]]`)

### Agent struct changes

```rust
pub struct Agent {
    pub name: String,
    pub description: String,
    pub source: Option<String>,
    pub requirements: Requirements,
    pub prompts: Option<Prompts>,
    pub executor: ExecutorConfig,
}

pub enum ExecutorConfig {
    Wasm(WasmConfig),
    Lambda(LambdaConfig),
    Container(ContainerConfig),
}

pub struct WasmConfig {
    pub module: PathBuf,
    pub mounts: Vec<Mount>,
}
```

No more `agent_dir` or `mounts` at the top level of `Agent`.

## Consequences

- **Breaking change** to manifest format — existing `agent.toml` files need migration
- **Clearer schema** — executor capabilities are explicit, not implicit
- **Validation catches mismatches** — can't add WASM-only features to Lambda agent
- **Agent struct is executor-agnostic** — WASM details live in `ExecutorConfig::Wasm`
- **Prepares for multi-executor future** — Lambda, containers become first-class
- **Migration path**: tool to convert old format → new format

## Migration

Old format:
```toml
name = "my-agent"
description = "..."
code = "agent.wasm"

[requirements.models]
phi3 = "file://./models/phi3.toml"

[[mounts]]
host_path = "mnt"
guest_path = "/"
mode = "rw"
```

New format:
```toml
name = "my-agent"
description = "..."
executor = "wasm"

[requirements.models]
phi3 = "file://./models/phi3.toml"

[wasm]
module = "agent.wasm"

[[wasm.mounts]]
host_path = "mnt"
guest_path = "/"
mode = "rw"
```

Automated migration: `vlinder migrate-manifest` command.
