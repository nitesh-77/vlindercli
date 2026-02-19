# =============================================================================
# Agent Builds (OCI container images via Podman)
# =============================================================================

# Build echo-container agent (OCI image via Podman)
build-echo-container:
    podman build -t localhost/echo-container:latest agents/echo-container/

# Build kv-bridge-agent (OCI image via Podman, exercises HTTP bridge)
build-kv-bridge-agent:
    podman build -t localhost/kv-bridge-agent:latest agents/kv-bridge-agent/

# Build todoapp agent (OCI image via Podman, OpenRouter integration test)
build-todoapp:
    podman build -t localhost/todoapp:latest agents/todoapp/

# Build pensieve-container (OCI image via Podman, full pensieve in Rust)
build-pensieve-container:
    podman build -t localhost/pensieve-container:latest agents/pensieve-container/

# Build support fleet agents (OCI images via Podman)
build-support-agent:
    podman build -t localhost/vlinder-support:latest agents/support-agent/

build-log-analyst:
    podman build -t localhost/vlinder-log-analyst:latest agents/log-analyst/

build-code-analyst:
    podman build -t localhost/vlinder-code-analyst:latest agents/code-analyst/

# Build all support fleet container images
build-support-fleet: build-support-agent build-log-analyst build-code-analyst

# =============================================================================
# Main Commands
# =============================================================================

# Build everything needed to run agents: CLI + all container images
build-everything: build build-todoapp build-echo-container

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
    cargo run -q -- model list

# Add a model from Ollama (usage: just model-add phi3)
model-add name:
    cargo run -q -- model add {{name}}

# Show registered models
model-registered:
    cargo run -q -- model registered

# Remove a registered model (usage: just model-remove phi3)
model-remove name:
    cargo run -q -- model remove {{name}}

# Run tests
test:
    cargo test

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
    find agents -name "*.db" -path "*/data/*" -delete
    find agents -name "*.db-shm" -path "*/data/*" -delete
    find agents -name "*.db-wal" -path "*/data/*" -delete

# Full reset: clean slate for testing. Preserves ~/.vlinder/config.toml only.
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

    # 2. Podman: stop and remove all vlinder containers and pods
    echo "  Cleaning Podman..."
    for pod in $(podman pod ls --format '{{{{.Name}}}}' 2>/dev/null | grep -i vlinder || true); do
        podman pod rm -f "$pod" 2>/dev/null || true
    done
    for ctr in $(podman ps -a --format '{{{{.Names}}}}' 2>/dev/null | grep -i vlinder || true); do
        podman rm -f "$ctr" 2>/dev/null || true
    done
    for img in $(podman images --format '{{{{.Repository}}}}:{{{{.Tag}}}}' 2>/dev/null | grep '^localhost/' || true); do
        podman rmi -f "$img" 2>/dev/null || true
    done
    podman system prune -f 2>/dev/null || true
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
    find agents -name "data" -type d -exec rm -rf {} + 2>/dev/null || true
    echo "  ✓ Agent data clean"

    # 6. Rust build artifacts
    echo "  Cleaning build artifacts..."
    cargo clean 2>/dev/null || true
    rm -rf agents/*/target
    echo "  ✓ Build artifacts clean"

    echo ""
    echo "=== Reset complete ==="
    echo "Preserved: ~/.vlinder/config.toml, ~/.vlinder/nats.conf"
    echo ""
    echo "Next steps:"
    echo "  1. Rebuild:  just build-everything"
    echo "  2. Daemon:   cargo run -- daemon"
    echo "  3. Run:      just run todoapp"

# Check license compliance (fails on GPL/copyleft)
license-check:
    cargo deny check licenses

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
    VLINDER_INTEGRATION_RUN="$RUN_DIR" cargo test --test '*' -- --ignored --test-threads=1

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
