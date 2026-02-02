# Test fixtures directory
fixtures := "tests/fixtures/agents"

# Dev project directory (for running agents during development)
dev_project := "dev-project/agents"

# Build all wasm agents and deploy to fixtures
build-agents:
    # Build agents
    cd agents/echo-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/upper-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/reverse-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/pensieve && cargo build --target wasm32-wasip1 --release

    # Copy WASM to source dirs (for running from agents/<name>/)
    cp agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm agents/echo-agent/agent.wasm
    cp agents/upper-agent/target/wasm32-unknown-unknown/release/upper_agent.wasm agents/upper-agent/agent.wasm
    cp agents/reverse-agent/target/wasm32-unknown-unknown/release/reverse_agent.wasm agents/reverse-agent/agent.wasm
    cp agents/pensieve/target/wasm32-wasip1/release/pensieve.wasm agents/pensieve/agent.wasm

    # Deploy to test fixtures (symlinks to source agents)
    mkdir -p {{fixtures}}/echo-agent
    mkdir -p {{fixtures}}/upper-agent
    mkdir -p {{fixtures}}/reverse-agent
    mkdir -p {{fixtures}}/pensieve/mnt
    mkdir -p {{fixtures}}/mount-test-agent/data
    mkdir -p {{fixtures}}/mount-test-agent/output
    mkdir -p {{fixtures}}/missing-mount-agent

    ln -sf ../../../../agents/echo-agent/agent.wasm {{fixtures}}/echo-agent/agent.wasm
    ln -sf ../../../../agents/echo-agent/agent.toml {{fixtures}}/echo-agent/agent.toml
    ln -sf ../../../../agents/upper-agent/agent.wasm {{fixtures}}/upper-agent/agent.wasm
    ln -sf ../../../../agents/upper-agent/agent.toml {{fixtures}}/upper-agent/agent.toml
    ln -sf ../../../../agents/reverse-agent/agent.wasm {{fixtures}}/reverse-agent/agent.wasm
    ln -sf ../../../../agents/reverse-agent/agent.toml {{fixtures}}/reverse-agent/agent.toml
    ln -sf ../../../../agents/pensieve/agent.wasm {{fixtures}}/pensieve/agent.wasm
    ln -sf ../../../../agents/pensieve/agent.toml {{fixtures}}/pensieve/agent.toml

    # mount-test-agent and missing-mount-agent use echo-agent wasm
    ln -sf ../../../../agents/echo-agent/agent.wasm {{fixtures}}/mount-test-agent/agent.wasm
    ln -sf ../../../../agents/mount-test-agent/agent.toml {{fixtures}}/mount-test-agent/agent.toml
    ln -sf ../../../../agents/echo-agent/agent.wasm {{fixtures}}/missing-mount-agent/agent.wasm
    ln -sf ../../../../agents/missing-mount-agent/agent.toml {{fixtures}}/missing-mount-agent/agent.toml

    # model-test-agent (for model loading tests, uses echo-agent wasm)
    mkdir -p {{fixtures}}/model-test-agent
    ln -sf ../../../../agents/echo-agent/agent.wasm {{fixtures}}/model-test-agent/agent.wasm
    ln -sf ../../../../agents/model-test-agent/agent.toml {{fixtures}}/model-test-agent/agent.toml
    ln -sf ../../../../agents/model-test-agent/models {{fixtures}}/model-test-agent/models

    # Setup fleet fixtures (symlinks to agent fixtures)
    mkdir -p tests/fixtures/fleets/test-fleet/agents
    ln -sf ../../../agents/echo-agent tests/fixtures/fleets/test-fleet/agents/echo-agent
    ln -sf ../../../agents/upper-agent tests/fixtures/fleets/test-fleet/agents/upper-agent

    # Deploy to dev-project (symlinks for development)
    mkdir -p {{dev_project}}/echo-agent
    mkdir -p {{dev_project}}/upper-agent
    mkdir -p {{dev_project}}/pensieve/mnt

    ln -sf ../../agents/echo-agent/agent.wasm {{dev_project}}/echo-agent/agent.wasm
    ln -sf ../../agents/echo-agent/agent.toml {{dev_project}}/echo-agent/agent.toml
    ln -sf ../../agents/upper-agent/agent.wasm {{dev_project}}/upper-agent/agent.wasm
    ln -sf ../../agents/upper-agent/agent.toml {{dev_project}}/upper-agent/agent.toml
    ln -sf ../../agents/pensieve/agent.wasm {{dev_project}}/pensieve/agent.wasm
    ln -sf ../../agents/pensieve/agent.toml {{dev_project}}/pensieve/agent.toml

# Build and deploy pensieve agent only
build-pensieve:
    cd agents/pensieve && cargo build --target wasm32-wasip1 --release
    # Copy to source dir (for running from agents/pensieve/)
    cp agents/pensieve/target/wasm32-wasip1/release/pensieve.wasm agents/pensieve/agent.wasm
    # Deploy to fixtures and dev-project (symlinks)
    mkdir -p {{fixtures}}/pensieve/mnt
    mkdir -p {{dev_project}}/pensieve/mnt
    ln -sf ../../../../agents/pensieve/agent.wasm {{fixtures}}/pensieve/agent.wasm
    ln -sf ../../../../agents/pensieve/agent.toml {{fixtures}}/pensieve/agent.toml
    ln -sf ../../agents/pensieve/agent.wasm {{dev_project}}/pensieve/agent.wasm
    ln -sf ../../agents/pensieve/agent.toml {{dev_project}}/pensieve/agent.toml

# Run a specific agent (usage: just run pensieve)
run agent:
    cargo run -- agent run -p {{dev_project}}/{{agent}}

# Run tests (builds agents first)
test: build-agents
    cargo test

# Build everything
build: build-agents
    cargo build

# Clean all build artifacts
clean:
    cargo clean
    rm -rf agents/*/target
    rm -rf {{fixtures}}
    rm -rf {{dev_project}}

# Check license compliance (fails on GPL/copyleft)
license-check:
    cargo deny check licenses
