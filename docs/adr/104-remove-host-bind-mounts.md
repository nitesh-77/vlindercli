# ADR 104: Remove Host Bind Mounts

## Status

Accepted

## Context

Agents declared `[[mounts]]` in their manifest to bind host directories into containers. The mechanism originated in ADR 016 (WASI preopened
directories), survived the move to OCI containers (ADR 046), and was wired through the full stack: manifest parsing, domain types, registry
persistence, gRPC proto, SQLite schema, and Podman runtime.

Two problems made it untenable:

1. **Incompatible with time travel.** Bind mounts give agents raw filesystem access. The sidecar (ADR 084) — which captures every side effect for snapshot and replay — cannot intercept writes to a mounted host directory. Any data written via mounts is invisible to the DAG, making time travel incomplete.

2. **Security inversion.** The CLI resolved relative paths in the manifest to absolute host paths and sent them to the daemon over gRPC. The agent author controlled which host directories were exposed. The daemon blindly trusted these paths. The person *operating* the platform should control host access, not the person *writing* the workload.

## Decision

Remove the host bind mount mechanism entirely. No replacement in this change — storage access will be redesigned as a first-class platform resource.

Removed:
- `Mount` domain type, `MountConfig` manifest type, `MountNotFound` error
- `resolve_host_path()` (tilde expansion, relative→absolute resolution)
- `ApiMount` struct (Podman REST API wire format)
- `mounts` parameter on the `Podman::run()` trait
- `Mount` protobuf message, `repeated Mount mounts` on `Agent` message
- `mounts_json` column in SQLite agents table
- Mount serialization in `StoredAgent` and `RegistryRepository`
- `dirs` dependency from vlinder-core (only used for mount path resolution)
- Test fixtures: `mount-test-agent`, `missing-mount-agent`
- `[[mounts]]` from all agent.toml files

## Future direction

Storage will be provided through the sidecar as S3-compatible object storage. Agents access storage via the sidecar's API, not the host filesystem. The sidecar captures every write, enabling time travel.

The design follows the same pattern as inference: agents declare requirements by name, operators register providers, the platform resolves at runtime.

- Agents declare volume requirements: `[requirements.volumes]`
- Operators register volumes: `vlinder volume add <name> --provider host|s3`
- Default provider is host-backed under `~/.vlinder/volumes/<name>/`
- S3-compatible providers (MinIO locally, AWS S3 in production) get
  versioning for free — snapshots are just version ID lists
- The sidecar proxies the S3 API, same as it proxies inference and KV

This connects to ADR 100 (storage traits as time travel contract).

## Consequences

- Agents that relied on mounts (`log-analyst`, `code-analyst`) lose host filesystem access until the volume mechanism is implemented.
- No data migration needed — mounts were runtime-only, not persisted state.
- The `reserved 10` in the proto preserves wire compatibility for the removed field number.
