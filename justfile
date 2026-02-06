# =============================================================================
# Agent Builds (OCI container images via Podman)
# =============================================================================

# Build echo-container agent (OCI image via Podman)
build-echo-container:
    podman build -t localhost/echo-container:latest agents/echo-container/

# Build kv-bridge-agent (OCI image via Podman, exercises HTTP bridge)
build-kv-bridge-agent:
    podman build -t localhost/kv-bridge-agent:latest agents/kv-bridge-agent/

# Build pensieve-container (OCI image via Podman, full pensieve in Rust)
build-pensieve-container:
    podman build -t localhost/pensieve-container:latest agents/pensieve-container/

# =============================================================================
# Main Commands
# =============================================================================

# Run a specific agent (usage: just run pensieve-container)
# Uses ~/.vlinder by default (no VLINDER_DIR override needed)
run agent:
    cargo build
    cd agents/{{agent}} && ../../target/debug/vlindercli agent run -p .

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

# Check license compliance (fails on GPL/copyleft)
license-check:
    cargo deny check licenses
