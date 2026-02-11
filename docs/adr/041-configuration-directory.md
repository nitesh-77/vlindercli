# ADR 041: Configuration Directory

## Status

Accepted (directory layout refined by ADR 066)

> **Note (post ADR 066):** The directory structure shown here (`config.toml`, `registry.db`, `agents/`, `models/`) has been refined by ADR 066 (platform root) which adds `conversations/`, `state/`, and `dag/` subdirectories. The config resolution mechanism (`VLINDER_DIR` env override, `~/.vlinder/` default) is unchanged.

## Context

Vlinder needs a consistent location for configuration and data:
- `config.toml` - user settings
- `registry.db` - registered models
- `agents/` - agent data/storage

Currently, `vlinder_dir()` defaults to `.vlinder` relative to the current working directory. This causes problems:
- Running from different directories finds different (or no) config
- `just run pensieve` changes CWD, breaking registry lookup
- Requires explicit `VLINDER_DIR` env var to work reliably

### Options Considered

**1. XDG Base Directory Specification**
```
~/.config/vlinder/config.toml
~/.local/share/vlinder/registry.db
~/.cache/vlinder/
```
- Pros: Linux standard, clean separation
- Cons: Split locations, macOS maps differently, not industry norm for infra tools

**2. Single Home Dotdir (`~/.vlinder/`)**
```
~/.vlinder/
├── config.toml
├── registry.db
├── agents/
└── models/
```
- Pros: Single location, matches kubectl/docker/terraform/aws pattern
- Cons: Not XDG compliant

**3. Keep Relative (`.vlinder/`)**
- Pros: Project-local by default
- Cons: Breaks when CWD changes, requires env var for reliability

### Industry Survey

| Tool | Location | Pattern |
|------|----------|---------|
| kubectl | `~/.kube/` | Home dotdir |
| Docker | `~/.docker/` | Home dotdir |
| Terraform | `~/.terraform.d/` | Home dotdir |
| AWS CLI | `~/.aws/` | Home dotdir |
| Ansible | `~/.ansible/` | Home dotdir |
| Helm | `~/.config/helm/` | XDG |

Most infrastructure tools use `~/.toolname/`. Only Helm fully adopted XDG.

## Decision

Use `~/.vlinder/` as the default base directory, with `VLINDER_DIR` env override.

### Directory Structure

```
~/.vlinder/
├── config.toml      # User configuration
├── registry.db      # Model registry (SQLite)
├── agents/          # Per-agent data
│   └── {name}/
│       └── {name}.db
└── models/          # Local model manifests (if any)
```

### Resolution Order

```rust
pub fn vlinder_dir() -> PathBuf {
    std::env::var("VLINDER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .expect("Could not determine home directory")
                .join(".vlinder")
        })
}
```

1. `VLINDER_DIR` environment variable (if set)
2. `~/.vlinder/` (default)

### Use Cases

**Normal usage:**
```bash
vlinder model add phi3        # Uses ~/.vlinder/registry.db
vlinder agent run pensieve    # Uses ~/.vlinder/
```

**Project-local (CI, testing, isolation):**
```bash
VLINDER_DIR=.vlinder vlinder model add phi3
# or
export VLINDER_DIR=/path/to/project/.vlinder
```

## Consequences

**Positive:**
- Single predictable location (`~/.vlinder/`)
- No CWD dependency - works from any directory
- Matches industry convention (kubectl, docker, terraform)
- Easy to backup/reset (`rm -rf ~/.vlinder`)
- Project-local still possible via `VLINDER_DIR`

**Negative:**
- Not XDG compliant (acceptable - neither are most infra tools)
- Global by default (some may prefer project-local)
- Existing `.vlinder/` in repo roots will be ignored (migration needed)

## Configuration Schema

```toml
# ~/.vlinder/config.toml

[logging]
level = "info"           # trace, debug, info, warn, error
llama_level = "error"    # llama.cpp/ggml noise suppression

[ollama]
endpoint = "http://localhost:11434"
```

## Layering: Env Overrides Config

Resolution order (highest priority first):
1. Environment variable
2. Config file (`~/.vlinder/config.toml`)
3. Default value

### Environment Variable Naming

Pattern: `VLINDER_<SECTION>_<KEY>` (uppercase, underscores)

| Config Key | Env Var |
|------------|---------|
| `logging.level` | `VLINDER_LOGGING_LEVEL` |
| `ollama.endpoint` | `VLINDER_OLLAMA_ENDPOINT` |

Special case: `VLINDER_DIR` overrides the base directory itself.

### Example Usage

```bash
# Override Ollama endpoint for remote server
VLINDER_OLLAMA_ENDPOINT=http://gpu-server:11434 vlinder model list

# Full isolation for testing
VLINDER_DIR=/tmp/test-vlinder vlinder model add phi3
```

## Migration

Users with existing `.vlinder/` directories in project roots:
1. Move to `~/.vlinder/`: `mv .vlinder ~/.vlinder`
2. Or set env: `export VLINDER_DIR=.vlinder`
