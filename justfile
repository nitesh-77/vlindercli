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

    # 4. Check container images
    if ! podman image exists localhost/echo-container:latest 2>/dev/null; then
        echo "  ✗ Missing image: localhost/echo-container:latest"
        echo "    Run: just build-echo-container"
        exit 1
    fi
    echo "  ✓ Container image: echo-container"

    # 5. Handle NATS
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

    # 6. Create date-stamped run directory
    RUN_DIR="/tmp/vlinder-integration/$(date +%Y-%m-%d-%H%M%S)"
    mkdir -p "$RUN_DIR"
    echo ""
    echo "=== Run directory: $RUN_DIR ==="
    echo ""

    # 7. Run integration tests
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
