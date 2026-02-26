# ADR 107: Read-Only S3-Compatible Mounts

**Status:** Draft

## Context

Agents run in containers. Today the only way to get data into an agent
is through HTTP APIs — provider hostnames for inference, sidecar
endpoints for storage. But many use cases need bulk read access to
files: documentation, source code, configuration, pre-built indexes.
Streaming these over HTTP at startup is wasteful and complex.

The support fleet (ADR 108) needs access to platform source and docs.
Other agents will need access to datasets, reference material, or
shared knowledge bases. The common need is: mount a bucket of files
into the container, read-only, at startup.

S3 is the universal object storage API. Every cloud provider implements
it (AWS, Cloudflare R2, MinIO, Garage, etc.). AWS's
`mountpoint-for-amazon-s3` (Apache 2.0) exposes an S3 bucket as a
POSIX filesystem — agents see files, not API calls.

## Decision

Agents declare S3 mounts in their manifest. The sidecar provisions
them at container startup using `mountpoint-for-amazon-s3`. The agent
sees a read-only directory of files.

```toml
[requirements.mounts]
knowledge = { s3 = "vlinder-support/v0.1.0/", path = "/knowledge" }
datasets  = { s3 = "team-datasets/current/", path = "/data", endpoint = "https://r2.example.com" }
```

Fields:

- `s3` — bucket name and optional prefix (e.g. `bucket/prefix/`)
- `path` — mount point inside the container
- `endpoint` — S3-compatible endpoint URL (optional; defaults to AWS)
- `secret` — reference to stored credentials (optional; for private
  buckets)

### What the agent sees

```
/knowledge/
  src/
    domain/
      agent.rs
      fleet.rs
      ...
  docs/
    adr/
      001-initial.md
      ...
```

Plain files. Standard `open()`, `read()`, `readdir()`. No SDK, no
client library, no API calls. Any language, any framework.

### What the sidecar manages

For each declared mount:

1. Resolve credentials (if `secret` is specified)
2. Start `mount-s3` with the bucket, prefix, and mount point
3. Bind-mount the FUSE mount into the container via Podman volume
4. Unmount on container stop

The mount lifecycle follows the container lifecycle.

### Read-only boundary

Mounts are read-only. `mountpoint-for-amazon-s3` supports read-only
mode natively — no write semantics to worry about, no conflict
resolution, no sync protocol. This is a deliberate product boundary.

Writable mounts, git-backed FUSE, bidirectional sync — all deferred to
future ADRs if and when a concrete use case demands them.

### Any S3-compatible backend

The manifest declares a bucket, not a provider. By defaulting to AWS
and allowing an `endpoint` override, the same manifest works with:

- AWS S3
- Cloudflare R2
- MinIO
- Garage
- Any S3-compatible service

No vendor lock-in at the manifest level.

## Consequences

- `agent.toml` gains `[requirements.mounts]` — new manifest schema
- Sidecar gains mount provisioning — new dependency on
  `mountpoint-for-amazon-s3` binary
- Host must support FUSE (standard on Linux, macFUSE on macOS)
- Podman volume support required for bind-mounting into containers
- Agents get bulk file access without HTTP streaming or custom clients
- Unblocks ADR 108: support fleet knowledge delivery via S3
- Read-only boundary keeps the implementation simple and auditable
- Future ADRs can extend to writable mounts, git-backed storage, or
  other mount types without changing the manifest schema shape
