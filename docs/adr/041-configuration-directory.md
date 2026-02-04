# ADR 041: Configuration Directory

## Status

Proposed

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

## Migration

Users with existing `.vlinder/` directories in project roots:
1. Move to `~/.vlinder/`: `mv .vlinder ~/.vlinder`
2. Or set env: `export VLINDER_DIR=.vlinder`
