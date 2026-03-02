# =============================================================================
# Platform Builds (OCI container images via Podman)
# =============================================================================

# Build vlinder-sidecar (OCI image via Podman, multi-stage Rust build)
build-sidecar:
    podman build -t localhost/vlinder-sidecar:latest -f crates/vlinder-sidecar/Dockerfile .

# =============================================================================
# Agent Builds (OCI container images via Podman)
# =============================================================================

# Build echo agent (OCI image via Podman, simplest possible agent)
build-echo:
    podman build -t localhost/echo:latest agents/echo/

# Build echo-container agent (OCI image via Podman)
build-echo-container:
    podman build -t localhost/echo-container:latest fleets/todoapp/agents/echo-container/

# Build openrouter-test agent (OCI image via Podman, exercises openrouter.vlinder.local)
build-openrouter-test:
    podman build -t localhost/openrouter-test:latest agents/openrouter-test/

# Build sqlite-kv-test agent (OCI image via Podman, exercises sqlite-kv.vlinder.local)
build-sqlite-kv-test:
    podman build -t localhost/sqlite-kv-test:latest agents/sqlite-kv-test/

# Build sqlite-vec-test agent (OCI image via Podman, exercises sqlite-vec.vlinder.local)
build-sqlite-vec-test:
    podman build -t localhost/sqlite-vec-test:latest agents/sqlite-vec-test/

# Build s3-mount-test agent (OCI image via Podman, exercises S3-backed FUSE mounts)
build-s3-mount-test:
    podman build -t localhost/s3-mount-test:latest agents/s3-mount-test/

# Build ollama-test agent (OCI image via Podman, exercises all four ollama.vlinder.local endpoints)
build-ollama-test:
    podman build -t localhost/ollama-test:latest agents/ollama-test/

# Build todoapp-orchestrator agent (OCI image via Podman, delegation smoke test)
build-todoapp-orchestrator:
    podman build -t localhost/todoapp-orchestrator:latest fleets/todoapp/agents/todoapp-orchestrator/

# Build all todoapp fleet container images
build-todoapp-fleet: build-todoapp-orchestrator build-echo-container

# Build todoapp agent (OCI image via Podman, OpenRouter integration test)
build-todoapp:
    podman build -t localhost/todoapp:latest agents/todoapp/

# Build pensieve-container (OCI image via Podman, full pensieve in Rust)
build-pensieve-container:
    podman build -t localhost/pensieve-container:latest agents/pensieve-container/

# Build support fleet agents (OCI images via Podman)
build-support-agent:
    podman build -t localhost/vlinder-support:latest fleets/support/agents/support-agent/

build-log-analyst:
    podman build -t localhost/vlinder-log-analyst:latest fleets/support/agents/log-analyst/

build-code-analyst:
    podman build -t localhost/vlinder-code-analyst:latest fleets/support/agents/code-analyst/

# Build all support fleet container images
build-support-fleet: build-support-agent build-log-analyst build-code-analyst

# Build council fleet agents (OCI images via Podman)
build-council-orchestrator:
    podman build -t localhost/council-orchestrator:latest fleets/council/agents/council-orchestrator/

build-sales-advisor:
    podman build -t localhost/council-sales-advisor:latest fleets/council/agents/sales-advisor/

build-product-advisor:
    podman build -t localhost/council-product-advisor:latest fleets/council/agents/product-advisor/

build-architect-advisor:
    podman build -t localhost/council-architect-advisor:latest fleets/council/agents/architect-advisor/

# Build all council fleet container images
build-council-fleet: build-council-orchestrator build-sales-advisor build-product-advisor build-architect-advisor

# =============================================================================
# Main Commands
# =============================================================================

# Build everything needed to run agents: CLI + sidecar + agent container images
build-everything: build build-sidecar build-todoapp build-todoapp-fleet build-council-fleet

# Run a specific agent (usage: just run pensieve-container)
# Uses ~/.vlinder by default (no VLINDER_DIR override needed)
run agent:
    cargo build
    cd agents/{{agent}} && ../../target/debug/vlinder agent run -p .

# Run the interactive support fleet
support:
    cargo build
    ./target/debug/vlinder support

# =============================================================================
# Model Catalog Commands
# =============================================================================

# List models from Ollama catalog
model-list:
    cargo run -q -p vlinder -- model list

# Add a model from Ollama (usage: just model-add phi3)
model-add name:
    cargo run -q -p vlinder -- model add {{name}}

# Show registered models
model-registered:
    cargo run -q -p vlinder -- model registered

# Remove a registered model (usage: just model-remove phi3)
model-remove name:
    cargo run -q -p vlinder -- model remove {{name}}

# Run tests
test:
    cargo test --workspace

# Run only podman-runtime crate tests (fast, no daemon deps)
test-podman-runtime:
    cargo test -p vlinder-podman-runtime

# Build CLI
build:
    cargo build

# Clean all build artifacts
clean:
    cargo clean
    rm -rf agents/*/target

# Clean data plane (DBs, WAL files, state pointers, conversations)
clean-data:
    rm -f ~/.vlinder/*.db ~/.vlinder/*.db-shm ~/.vlinder/*.db-wal
    rm -rf ~/.vlinder/conversations
    rm -rf ~/.vlinder/state
    find agents fleets -name "*.db" -path "*/data/*" -delete
    find agents fleets -name "*.db-shm" -path "*/data/*" -delete
    find agents fleets -name "*.db-wal" -path "*/data/*" -delete

# Full reset: clean slate for testing. Preserves config, credentials, and nats.conf.
reset:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Vlinder Reset ==="

    # 1. Stop daemon/worker processes
    if pgrep -f "vlinder.*daemon" >/dev/null 2>&1 || pgrep -f "VLINDER_WORKER_ROLE" >/dev/null 2>&1; then
        echo "  Stopping vlinder processes..."
        pkill -f "VLINDER_WORKER_ROLE" 2>/dev/null || true
        pkill -f "vlinder.*daemon" 2>/dev/null || true
        sleep 1
        echo "  ✓ Processes stopped"
    else
        echo "  - No vlinder processes running"
    fi

    # 2. Podman: nuke everything (this machine only uses Podman for Vlinder)
    echo "  Cleaning Podman..."
    podman pod rm -a -f 2>/dev/null || true
    podman rm -a -f 2>/dev/null || true
    podman rmi -a -f 2>/dev/null || true
    podman system prune -a -f --volumes 2>/dev/null || true
    echo "  ✓ Podman clean"

    # 3. NATS JetStream streams
    if command -v nats >/dev/null 2>&1 && nc -z localhost 4222 2>/dev/null; then
        echo "  Cleaning NATS streams..."
        for stream in $(nats stream ls --names 2>/dev/null || true); do
            nats stream rm -f "$stream" 2>/dev/null || true
        done
        echo "  ✓ NATS streams purged"
    else
        echo "  - NATS not reachable, skipping stream cleanup"
    fi

    # 4. ~/.vlinder data plane (preserve config.toml and nats.conf)
    echo "  Cleaning ~/.vlinder data..."
    rm -f ~/.vlinder/*.db ~/.vlinder/*.db-shm ~/.vlinder/*.db-wal
    rm -rf ~/.vlinder/conversations
    rm -rf ~/.vlinder/state
    rm -rf ~/.vlinder/logs
    rm -rf ~/.vlinder/nats-data
    echo "  ✓ Data plane clean"

    # 5. Agent-local data directories
    echo "  Cleaning agent data..."
    find agents fleets -name "data" -type d -exec rm -rf {} + 2>/dev/null || true
    echo "  ✓ Agent data clean"

    # 6. Rust build artifacts
    echo "  Cleaning build artifacts..."
    cargo clean 2>/dev/null || true
    rm -rf agents/*/target fleets/*/agents/*/target
    echo "  ✓ Build artifacts clean"

    echo ""
    echo "=== Reset complete ==="
    echo "Preserved: ~/.vlinder/config.toml, ~/.vlinder/client.toml, ~/.vlinder/nats.conf, ~/.vlinder/*.creds"
    echo ""
    echo "Next steps:"
    echo "  1. Rebuild:  just build-everything"
    echo "  2. Daemon:   cargo run -p vlindercli --bin vlinderd"
    echo "  3. Run:      just run todoapp"

# Check license compliance (fails on GPL/copyleft)
license-check:
    cargo deny check licenses

# =============================================================================
# Local S3 (LocalStack + s3fs-fuse)
#
# S3 mounts (ADR 107) use s3fs-fuse to present S3 bucket prefixes as
# read-only FUSE filesystems inside agent containers. On macOS this
# requires setup inside the Podman Machine VM (CoreOS on Apple HV):
#
# Chain: s3-install → s3-setup → s3-start → s3-seed
#
# s3-install:  rpm-ostree install s3fs-fuse into the CoreOS VM.
#              CoreOS is immutable — rpm-ostree layers packages on top
#              and requires a reboot (machine stop/start) to apply.
#              The COPR repo for podman-release can cause GPG key 404s;
#              if that happens: podman machine ssh "sudo mv
#              /etc/yum.repos.d/podman-release-copr.repo /tmp/"
#
# s3-setup:   Symlink mount.fuse.s3fs → s3fs in /usr/sbin.
#              The `mount` command searches /sbin and /usr/sbin for
#              helpers named mount.<type>. CoreOS /usr/sbin is normally
#              read-only, but `sudo ln -sf` works through the composefs
#              overlay. /usr/local/sbin does NOT work (not in mount's
#              search path).
#
# s3-start:   Run LocalStack in a Podman container. Idempotent: skips
#              if already running, removes stale stopped container first.
#              IMPORTANT: LocalStack stops when the Podman machine
#              restarts (stop/start or reset). If s3fs mounts hang after
#              a machine restart, check `podman ps -a` — LocalStack is
#              probably "Exited". Run `just s3-start` to bring it back.
#
# s3-seed:    Populate the test bucket with sample data. Creates a
#              zero-byte directory marker at each prefix path — s3fs 1.97
#              crashes without this (see note in s3-seed below).
# =============================================================================

# Install s3fs-fuse in the Podman VM (requires machine restart)
# CoreOS uses rpm-ostree which layers packages immutably; reboot needed to apply.
s3-install:
    #!/usr/bin/env bash
    set -euo pipefail
    if podman machine ssh "command -v s3fs" >/dev/null 2>&1; then
        echo "  ✓ s3fs-fuse already installed"
    else
        echo "  Installing s3fs-fuse (rpm-ostree)..."
        podman machine ssh "sudo rpm-ostree install s3fs-fuse"
        echo "  Restarting Podman machine (CoreOS needs reboot for rpm-ostree)..."
        podman machine stop
        podman machine start
        echo "  ✓ s3fs-fuse installed"
    fi

# Symlink the s3fs mount helper so `mount -t fuse.s3fs` finds it.
# The mount command looks for /usr/sbin/mount.<type> — on CoreOS the
# s3fs binary lands in /usr/bin but mount won't find it there.
# `sudo ln -sf` works through CoreOS's composefs overlay even though
# /usr/sbin appears read-only (cp fails, but symlink works).
s3-setup: s3-install
    #!/usr/bin/env bash
    set -euo pipefail
    if podman machine ssh "test -f /usr/sbin/mount.fuse.s3fs" 2>/dev/null; then
        echo "  ✓ mount.fuse.s3fs already installed"
    else
        podman machine ssh "sudo ln -sf /usr/bin/s3fs /usr/sbin/mount.fuse.s3fs"
        echo "  ✓ mount.fuse.s3fs symlinked in /usr/sbin"
    fi

# Start LocalStack for local S3 development (idempotent).
# Handles three states: running (skip), stopped/stale (rm + create), absent (create).
# Uses --filter + --quiet instead of --format '{{ "{{" }}.Names{{ "}}" }}'
# because {{ is justfile interpolation syntax.
s3-start: s3-setup
    #!/usr/bin/env bash
    set -euo pipefail
    if podman ps --filter name=^localstack$ --quiet 2>/dev/null | grep -q .; then
        echo "  ✓ LocalStack already running"
    else
        podman rm -f localstack 2>/dev/null || true
        podman run -d --name localstack -p 4566:4566 localstack/localstack:latest
        echo "  LocalStack S3 running at http://localhost:4566"
    fi

# Stop and remove LocalStack
s3-stop:
    podman rm -f localstack 2>/dev/null || true

# Create a test bucket and upload sample data.
#
# IMPORTANT: creates a zero-byte directory marker at the prefix path.
# s3fs 1.97 has a bug in remote_mountpath_exists: when mounting a sub-path
# (e.g., bucket:/v0.1.0/), it does HEAD on the prefix key. If that returns
# 404 (no directory marker), s3fs crashes with:
#   basic_string::back() Assertion '!empty()' failed
# This kills the FUSE daemon, leaving a stale mount point that reports
# "Transport endpoint is not connected" for all access attempts.
# AWS S3 Console creates these markers when you "create a folder" —
# LocalStack and other S3-compatible stores do not create them automatically.
#
# The `|| true` on `s3 ls` prevents broken pipe (SIGPIPE/exit 120) when
# `head` closes the pipe before `aws s3 ls` finishes writing.
s3-seed:
    #!/usr/bin/env bash
    set -euo pipefail
    export AWS_ACCESS_KEY_ID=test
    export AWS_SECRET_ACCESS_KEY=test
    export AWS_DEFAULT_REGION=us-east-1
    aws --endpoint-url http://localhost:4566 s3 mb s3://vlinder-support 2>/dev/null || true
    # Directory marker — without this, s3fs 1.97 crashes on sub-path mounts
    aws --endpoint-url http://localhost:4566 s3api put-object --bucket vlinder-support --key "v0.1.0/" --content-length 0 >/dev/null
    echo "The mount works." | aws --endpoint-url http://localhost:4566 s3 cp - s3://vlinder-support/v0.1.0/probe.txt
    aws --endpoint-url http://localhost:4566 s3 sync docs/ s3://vlinder-support/v0.1.0/docs/
    aws --endpoint-url http://localhost:4566 s3 sync crates/vlinder-core/src/ s3://vlinder-support/v0.1.0/src/
    echo "Seeded s3://vlinder-support/v0.1.0/"
    aws --endpoint-url http://localhost:4566 s3 ls s3://vlinder-support/v0.1.0/ --recursive | head -20 || true

# =============================================================================
# Integration Tests (ADR 082)
# =============================================================================

# Run all integration tests with prerequisite checking and isolation
run-integration-tests:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Checking prerequisites ==="

    # 1. Check required binaries
    missing=()
    command -v podman >/dev/null 2>&1 || missing+=("podman — install from https://podman.io")
    command -v nats-server >/dev/null 2>&1 || missing+=("nats-server — brew install nats-server")
    command -v ollama >/dev/null 2>&1 || missing+=("ollama — install from https://ollama.com")

    if [ ${#missing[@]} -gt 0 ]; then
        echo "Missing prerequisites:"
        for m in "${missing[@]}"; do
            echo "  ✗ $m"
        done
        exit 1
    fi
    echo "  ✓ podman, nats-server, ollama"

    # 2. Verify Ollama endpoint responds
    if ! curl -sf http://localhost:11434/api/tags >/dev/null 2>&1; then
        echo "  ✗ Ollama not responding on localhost:11434"
        echo "    Run: ollama serve"
        exit 1
    fi
    echo "  ✓ Ollama endpoint"

    # 3. Check required models (if any listed in tests/required-models.txt)
    if [ -f tests/required-models.txt ]; then
        while IFS= read -r model || [ -n "$model" ]; do
            # Skip comments and empty lines
            [[ "$model" =~ ^#.*$ || -z "$model" ]] && continue
            if ! ollama list 2>/dev/null | grep -q "^${model}"; then
                echo "  ✗ Missing model: $model"
                echo "    Run: ollama pull $model"
                exit 1
            fi
            echo "  ✓ Model: $model"
        done < tests/required-models.txt
    fi

    # 4. Check OpenRouter API key (optional — tests skip gracefully if absent)
    if [ -n "${VLINDER_OPENROUTER_API_KEY:-}" ]; then
        echo "  ✓ VLINDER_OPENROUTER_API_KEY set — OpenRouter tests will run"
    else
        echo "  ⚠ VLINDER_OPENROUTER_API_KEY not set — OpenRouter tests will be skipped"
        echo "    To enable: export VLINDER_OPENROUTER_API_KEY=<your-key>"
    fi

    # 5. Check container images
    if ! podman image exists localhost/echo-container:latest 2>/dev/null; then
        echo "  ✗ Missing image: localhost/echo-container:latest"
        echo "    Run: just build-echo-container"
        exit 1
    fi
    echo "  ✓ Container image: echo-container"

    # 6. Handle NATS
    echo ""
    echo "=== Setting up NATS ==="

    # Stop any existing NATS we started
    if [ -f /tmp/vlinder-nats/nats.pid ]; then
        old_pid=$(cat /tmp/vlinder-nats/nats.pid 2>/dev/null || true)
        if [ -n "$old_pid" ] && kill -0 "$old_pid" 2>/dev/null; then
            echo "  Stopping previous NATS (pid $old_pid)"
            kill "$old_pid" 2>/dev/null || true
            sleep 1
        fi
    fi

    # Clean JetStream state
    rm -rf /tmp/vlinder-nats
    mkdir -p /tmp/vlinder-nats

    # Start NATS fresh
    nats-server -js -sd /tmp/vlinder-nats -p 4222 --pid /tmp/vlinder-nats/nats.pid &
    NATS_PID=$!
    echo "  Started NATS (pid $NATS_PID)"

    # Cleanup trap
    cleanup() {
        echo ""
        echo "=== Stopping NATS (pid $NATS_PID) ==="
        kill "$NATS_PID" 2>/dev/null || true
    }
    trap cleanup EXIT

    # Wait for NATS to be ready
    for i in $(seq 1 30); do
        if nc -z localhost 4222 2>/dev/null; then
            break
        fi
        sleep 0.1
    done
    if ! nc -z localhost 4222 2>/dev/null; then
        echo "  ✗ NATS failed to start on localhost:4222"
        exit 1
    fi
    echo "  ✓ NATS ready on localhost:4222"

    # 7. Create date-stamped run directory
    RUN_DIR="/tmp/vlinder-integration/$(date +%Y-%m-%d-%H%M%S)"
    mkdir -p "$RUN_DIR"
    echo ""
    echo "=== Run directory: $RUN_DIR ==="
    echo ""

    # 8. Run integration tests
    echo "=== Running integration tests ==="
    VLINDER_INTEGRATION_RUN="$RUN_DIR" cargo test --features test-support --test '*' -- --ignored --test-threads=1

    echo ""
    echo "=== Done. Artifacts in: $RUN_DIR ==="

# Generate a diagnostic prompt from a failed integration test directory
debug-integration-test path:
    #!/usr/bin/env bash
    set -euo pipefail

    TEST_DIR="{{path}}"
    VLINDER="$TEST_DIR/.vlinder"

    if [ ! -d "$VLINDER" ]; then
        echo "No .vlinder directory found at: $VLINDER"
        echo "Usage: just debug-integration-test /tmp/vlinder-integration/<run>/<test_name>"
        exit 1
    fi

    echo "=== Integration Test Diagnostic ==="
    echo "Test directory: $TEST_DIR"
    echo ""

    # Conversations git log
    if [ -d "$VLINDER/conversations/.git" ]; then
        echo "--- Conversation History ---"
        git -C "$VLINDER/conversations" log --oneline --all --graph 2>/dev/null || echo "(no commits)"
        echo ""
        echo "--- Last Commit Details ---"
        git -C "$VLINDER/conversations" log -1 --format=full 2>/dev/null || echo "(no commits)"
        echo ""
        echo "--- Branches ---"
        git -C "$VLINDER/conversations" branch -a 2>/dev/null || echo "(no branches)"
        echo ""
    else
        echo "--- No conversation repo ---"
        echo ""
    fi

    # Logs
    if [ -d "$VLINDER/logs" ]; then
        echo "--- Logs ---"
        for f in "$VLINDER/logs"/*; do
            [ -f "$f" ] || continue
            echo "[$f]"
            tail -50 "$f"
            echo ""
        done
    fi

    # Config
    if [ -f "$VLINDER/config.toml" ]; then
        echo "--- Config ---"
        cat "$VLINDER/config.toml"
        echo ""
    fi

    # Database files
    echo "--- Database Files ---"
    find "$VLINDER" -name "*.db" -exec ls -lh {} \; 2>/dev/null || echo "(none)"
    echo ""

    echo "--- Directory Tree ---"
    find "$TEST_DIR" -maxdepth 4 -type f | head -50
    echo ""
    echo "=== End Diagnostic ==="
