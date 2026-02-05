# =============================================================================
# Agent Builds (only agents whose WASM is actually executed in tests)
# =============================================================================

# Build echo-agent
# Required by: tests/wasm_tests.rs (agent_echo)
build-echo-agent:
    cd agents/echo-agent && cargo build --target wasm32-unknown-unknown --release
    cp agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm agents/echo-agent/echo-agent.wasm
    cp agents/echo-agent/echo-agent.wasm tests/fixtures/agents/echo-agent/echo-agent.wasm

# Build upper-agent
# Required by: tests/wasm_tests.rs (agent_upper)
build-upper-agent:
    cd agents/upper-agent && cargo build --target wasm32-unknown-unknown --release
    cp agents/upper-agent/target/wasm32-unknown-unknown/release/upper_agent.wasm agents/upper-agent/upper-agent.wasm
    cp agents/upper-agent/upper-agent.wasm tests/fixtures/agents/upper-agent/upper-agent.wasm

# Build reverse-agent
# Required by: tests/daemon_tests.rs (daemon_invokes_agent_and_returns_result)
#              tests/wasm_runtime_tests.rs (runtime_executes_agent_and_returns_response)
build-reverse-agent:
    cd agents/reverse-agent && cargo build --target wasm32-unknown-unknown --release
    cp agents/reverse-agent/target/wasm32-unknown-unknown/release/reverse_agent.wasm agents/reverse-agent/reverse-agent.wasm
    cp agents/reverse-agent/reverse-agent.wasm tests/fixtures/agents/reverse-agent/reverse-agent.wasm

# Build pensieve agent (uses wasm32-wasip1 for WASI support)
build-pensieve:
    cd agents/pensieve && cargo build --target wasm32-wasip1 --release
    cp agents/pensieve/target/wasm32-wasip1/release/pensieve.wasm agents/pensieve/pensieve.wasm

# =============================================================================
# Main Commands
# =============================================================================

# Run a specific agent (usage: just run pensieve)
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
test: build-echo-agent build-upper-agent build-reverse-agent
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
