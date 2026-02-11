# ADR 070: SubmissionId Without Git

## Status

Accepted

## Context

The harness generates a SubmissionId by committing user input to git and using the commit SHA (ADR 054). This couples identity generation to the ConversationStore — the harness must shell out to `git commit` before it can send an invoke to NATS.

This creates three problems:

**The GitDagWorker can't take over.** ADR 065 says git writing should be an async NATS projection. ADR 069 says commits should use agent identity as author. But the ConversationStore runs synchronously in the harness, before anything hits NATS, using the user's personal git identity. The GitDagWorker was built to replace it but can't — the harness needs the SHA before it can send the invoke.

**The harness knows about git.** The harness deploys agents and manages invocations. It shouldn't need to know about git repos, commit SHAs, or JSON serialization. Storage concerns leak into the control plane.

**Two writers to one repo.** If the GitDagWorker runs alongside the ConversationStore, both write to `~/.vlinder/conversations/` — conflicting commits, different authors, different formats.

### What git actually does

A git commit SHA is `SHA-1("commit <size>\0<commit-object>")`. The commit object is:

```
tree <tree-hash>
parent <parent-hash>
author <name> <email> <timestamp> <tz>
committer <name> <email> <timestamp> <tz>

<commit message>
```

The inputs are: file contents (via tree/blob hashes), parent commit, author identity, timestamp, and message. We can compute this in-process without git.

## Decision

**The harness computes the SubmissionId as a SHA-1 using git's object format, without shelling out to git.** The ConversationStore is removed from the harness. The GitDagWorker becomes the sole writer to `~/.vlinder/conversations/`.

### SubmissionId generation

The harness builds the same byte sequence git would produce — blob hash from payload, tree hash from blob, commit object from tree + parent + author + message — and SHA-1s it. The result is the SubmissionId.

```rust
fn compute_submission_id(
    payload: &[u8],
    parent_hash: &str,
    author: &str,
    timestamp: i64,
    message: &str,
) -> String {
    let blob_hash = git_hash_object("blob", payload);
    let tree = format!("100644 payload\0{}", blob_hash);
    let tree_hash = git_hash_object("tree", tree.as_bytes());

    let commit = format!(
        "tree {}\nparent {}\nauthor {} {} +0000\ncommitter {} {} +0000\n\n{}\n",
        tree_hash, parent_hash, author, timestamp, author, timestamp, message,
    );
    git_hash_object("commit", commit.as_bytes())
}

fn git_hash_object(object_type: &str, content: &[u8]) -> String {
    let header = format!("{} {}\0", object_type, content.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(content);
    hex::encode(hasher.finalize())
}
```

The SubmissionId is git-compatible. When the GitDagWorker later writes the actual commit with the same content, it produces the same SHA.

### What changes

- The harness no longer holds a `ConversationStore`. It computes the SubmissionId in-process and sends the invoke to NATS.
- The GitDagWorker (ADR 065) subscribes to NATS, receives the invoke, and writes the git commit to `~/.vlinder/conversations/` with agent identity as author (ADR 069).
- One writer to the conversations repo. No conflicts.
- The `ConversationStore` type is removed. Its read operations (`load`, `read_state_trailer`, `latest_state_for_agent`) move to the GitDagWorker or become git queries via `vlinder timeline` (ADR 068).

### Commit tree format

Each GitDagWorker commit is a system snapshot:

```
tree
├── payload              (message content)
├── agent.toml           (agent manifest from registry at time of message)
├── models/
│   ├── <model-name>.toml  (each model the agent requires)
│   └── ...
└── platform.toml        (vlinder version, source commit SHA, registry host)
```

`platform.toml` contents:
```toml
version = "0.1.0"
commit = "7b08ef1a3c..."    # or "unknown" if not built from git
registry_host = "localhost"
```

Git deduplicates unchanged blobs: if agent.toml hasn't changed between messages, the blob SHA is identical — zero extra storage. The GitDagWorker runs async off NATS (ADR 065), so tree building has no impact on agent execution.

### Build-time version embedding

`env!("VLINDER_GIT_SHA")` is embedded at compile time via `build.rs` with fallback chain:
1. `VLINDER_BUILD_SHA` env var (set in CI/packaging scripts)
2. `git rev-parse HEAD` + `-dirty` suffix if working tree is dirty
3. `"unknown"` fallback (e.g., crates.io install with no `.git`)

### What doesn't change

- The SubmissionId is still a content-addressed SHA-1 — same algorithm, same inputs, same output.
- The conversations repo at `~/.vlinder/conversations/` stays.
- The commit format (message, trailers) stays.
- `vlinder timeline` still works — it reads git, doesn't care who wrote the commits.

## Consequences

- The harness has no git dependency — it computes a hash and sends a NATS message
- The GitDagWorker is the sole writer to the conversations repo
- Agent identity (ADR 069) is applied consistently — every commit has the correct author
- No two writers contend on the same repo
- SubmissionId computation is pure — no I/O, no subprocess, deterministic
- The ConversationStore type is removed; its responsibilities split between in-process hashing (identity) and the GitDagWorker (persistence)
