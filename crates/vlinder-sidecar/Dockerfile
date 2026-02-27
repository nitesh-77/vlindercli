# Multi-stage build for vlinder-sidecar.
# Build context: workspace root (sidecar depends on sibling crates).
#
# Usage:
#   podman build -t localhost/vlinder-sidecar:latest -f crates/vlinder-sidecar/Dockerfile .

# ── Build stage ──────────────────────────────────────────────────────
FROM rust:1.90-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace manifest and all crate manifests first (layer caching)
COPY Cargo.toml ./
COPY crates/vlinder-core/Cargo.toml crates/vlinder-core/
COPY crates/vlinder-proto/Cargo.toml crates/vlinder-proto/
COPY crates/vlinder-ollama/Cargo.toml crates/vlinder-ollama/
COPY crates/vlinder-infer-openrouter/Cargo.toml crates/vlinder-infer-openrouter/
COPY crates/vlinder-sqlite-vec/Cargo.toml crates/vlinder-sqlite-vec/
COPY crates/vlinder-sqlite-kv/Cargo.toml crates/vlinder-sqlite-kv/
COPY crates/vlinder-nats/Cargo.toml crates/vlinder-nats/
COPY crates/vlinder-sidecar/Cargo.toml crates/vlinder-sidecar/
COPY crates/vlinder-podman-runtime/Cargo.toml crates/vlinder-podman-runtime/
COPY crates/vlinder-catalog/Cargo.toml crates/vlinder-catalog/
COPY crates/vlinder-sql-state/Cargo.toml crates/vlinder-sql-state/
COPY crates/vlinderd/Cargo.toml crates/vlinderd/
COPY crates/vlinder/Cargo.toml crates/vlinder/

# Stub out src directories so cargo can resolve the workspace
RUN mkdir -p crates/vlinder-core/src && echo "" > crates/vlinder-core/src/lib.rs && \
    mkdir -p crates/vlinder-ollama/src && echo "" > crates/vlinder-ollama/src/lib.rs && \
    mkdir -p crates/vlinder-infer-openrouter/src && echo "" > crates/vlinder-infer-openrouter/src/lib.rs && \
    mkdir -p crates/vlinder-sqlite-vec/src && echo "" > crates/vlinder-sqlite-vec/src/lib.rs && \
    mkdir -p crates/vlinder-sqlite-kv/src && echo "" > crates/vlinder-sqlite-kv/src/lib.rs && \
    mkdir -p crates/vlinder-proto/src && echo "" > crates/vlinder-proto/src/lib.rs && \
    mkdir -p crates/vlinder-nats/src && echo "" > crates/vlinder-nats/src/lib.rs && \
    mkdir -p crates/vlinder-podman-runtime/src && echo "" > crates/vlinder-podman-runtime/src/lib.rs && \
    mkdir -p crates/vlinder-catalog/src && echo "" > crates/vlinder-catalog/src/lib.rs && \
    mkdir -p crates/vlinder-sql-state/src && echo "" > crates/vlinder-sql-state/src/lib.rs && \
    mkdir -p crates/vlinder-sidecar/src && echo "fn main() {}" > crates/vlinder-sidecar/src/main.rs && \
    mkdir -p crates/vlinderd/src && echo "fn main() {}" > crates/vlinderd/src/main.rs && \
    mkdir -p crates/vlinder/src && echo "fn main() {}" > crates/vlinder/src/main.rs

# Proto build dependencies (proto files needed for build.rs)
COPY crates/vlinder-proto/build.rs crates/vlinder-proto/
COPY crates/vlinder-proto/proto crates/vlinder-proto/proto
COPY crates/vlinder-catalog/build.rs crates/vlinder-catalog/
COPY crates/vlinder-catalog/proto crates/vlinder-catalog/proto
COPY crates/vlinder-sql-state/build.rs crates/vlinder-sql-state/
COPY crates/vlinder-sql-state/proto crates/vlinder-sql-state/proto

# Pre-build dependencies (this layer is cached unless Cargo.toml changes)
RUN cargo build --release -p vlinder-sidecar 2>/dev/null || true

# Copy actual source code
COPY crates/vlinder-core/src crates/vlinder-core/src
COPY crates/vlinder-ollama/src crates/vlinder-ollama/src
COPY crates/vlinder-infer-openrouter/src crates/vlinder-infer-openrouter/src
COPY crates/vlinder-sqlite-vec/src crates/vlinder-sqlite-vec/src
COPY crates/vlinder-sqlite-kv/src crates/vlinder-sqlite-kv/src
COPY crates/vlinder-proto/src crates/vlinder-proto/src
COPY crates/vlinder-nats/src crates/vlinder-nats/src
COPY crates/vlinder-podman-runtime/src crates/vlinder-podman-runtime/src
COPY crates/vlinder-catalog/src crates/vlinder-catalog/src
COPY crates/vlinder-sql-state/src crates/vlinder-sql-state/src
COPY crates/vlinder-sidecar/src crates/vlinder-sidecar/src

# Touch source files to invalidate the cache for actual compilation
RUN touch crates/vlinder-core/src/lib.rs \
          crates/vlinder-ollama/src/lib.rs \
          crates/vlinder-infer-openrouter/src/lib.rs \
          crates/vlinder-sqlite-vec/src/lib.rs \
          crates/vlinder-sqlite-kv/src/lib.rs \
          crates/vlinder-nats/src/lib.rs \
          crates/vlinder-podman-runtime/src/lib.rs \
          crates/vlinder-proto/src/lib.rs \
          crates/vlinder-catalog/src/lib.rs \
          crates/vlinder-sql-state/src/lib.rs \
          crates/vlinder-sidecar/src/main.rs

# Build the sidecar binary
RUN cargo build --release -p vlinder-sidecar

# ── Runtime stage ────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/vlinder-sidecar /usr/local/bin/vlinder-sidecar

ENTRYPOINT ["vlinder-sidecar"]
