# ADR 015: Vlinderfile Is Agent Manifest

## Status

Accepted (updates ADR 011)

## Context

ADR 011 established that agents declare models upfront, validated at inference time. However:
1. Model declarations were passed to `Agent::load()` in Rust code
2. No standard way to declare agent identity, description, or capabilities
3. Prompt customization required recompiling the wasm

We need a manifest that:
- Declares agent identity and requirements
- Enables prompt customization without recompilation
- Serves as source of truth for both runtime and plugin

## Decision

**Vlinderfile is the agent, serialized as TOML.**

```toml
name = "pensieve"
description = "A memory system for web articles"
source = "agents/pensieve"

[requirements]
models = ["phi3", "nomic-embed"]
host_capabilities = ["fetch_url", "store_file", "embed", "search_by_vector", "infer"]

[prompts]
# Optional overrides - defaults compiled into wasm
intent_recognition = '''
...custom prompt...
'''
```

**Agent struct is the deserialized Vlinderfile:**

```rust
#[derive(Deserialize)]
pub struct Agent {
    pub name: String,
    pub description: String,
    pub source: Option<String>,
    pub requirements: Requirements,
    pub prompts: Option<Prompts>,

    #[serde(skip)]
    wasm_path: String,      // derived from name
    #[serde(skip)]
    vlinderfile_raw: String, // for passing to plugin
}

// Load is just deserialization + derived fields
let agent = Agent::load("pensieve")?;
```

**Plugin accesses Vlinderfile via host function:**

```rust
// In wasm plugin
#[host_fn]
extern "ExtismHost" {
    pub fn get_vlinderfile() -> String;
}

// Use with fallback to compiled defaults
let template = get_prompt(
    |p: &Prompts| &p.intent_recognition,
    DEFAULT_INTENT_RECOGNITION,  // include_str!("prompts/intent.txt")
);
```

## Consequences

- `Agent::load(name)` takes only name — everything else from Vlinderfile
- Model validation uses `requirements.models` from Vlinderfile
- Prompts customizable at runtime without recompilation
- Defaults compiled into wasm via `include_str!` ensure agent always works
- `get_default_config` export provides reset capability via minijinja templating
- Clear separation: Vlinderfile for configuration, wasm for behavior
