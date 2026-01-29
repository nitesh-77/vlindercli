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

    # Deploy to test fixtures (agent.toml + agent.wasm convention)
    mkdir -p {{fixtures}}/echo-agent
    mkdir -p {{fixtures}}/upper-agent
    mkdir -p {{fixtures}}/reverse-agent
    mkdir -p {{fixtures}}/pensieve/mnt
    mkdir -p {{fixtures}}/mount-test-agent/data
    mkdir -p {{fixtures}}/mount-test-agent/output
    mkdir -p {{fixtures}}/missing-mount-agent

    cp agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm {{fixtures}}/echo-agent/agent.wasm
    cp agents/echo-agent/agent.toml {{fixtures}}/echo-agent/
    cp agents/upper-agent/target/wasm32-unknown-unknown/release/upper_agent.wasm {{fixtures}}/upper-agent/agent.wasm
    cp agents/upper-agent/agent.toml {{fixtures}}/upper-agent/
    cp agents/reverse-agent/target/wasm32-unknown-unknown/release/reverse_agent.wasm {{fixtures}}/reverse-agent/agent.wasm
    cp agents/reverse-agent/agent.toml {{fixtures}}/reverse-agent/
    cp agents/pensieve/target/wasm32-wasip1/release/pensieve.wasm {{fixtures}}/pensieve/agent.wasm
    cp agents/pensieve/agent.toml {{fixtures}}/pensieve/

    # mount-test-agent and missing-mount-agent use echo-agent wasm
    cp {{fixtures}}/echo-agent/agent.wasm {{fixtures}}/mount-test-agent/agent.wasm
    cp agents/mount-test-agent/agent.toml {{fixtures}}/mount-test-agent/
    cp {{fixtures}}/echo-agent/agent.wasm {{fixtures}}/missing-mount-agent/agent.wasm
    cp agents/missing-mount-agent/agent.toml {{fixtures}}/missing-mount-agent/

    # Deploy to dev-project (for vlinder -p dev-project/agents/pensieve)
    mkdir -p {{dev_project}}/echo-agent
    mkdir -p {{dev_project}}/upper-agent
    mkdir -p {{dev_project}}/pensieve/mnt

    cp {{fixtures}}/echo-agent/agent.wasm {{dev_project}}/echo-agent/
    cp {{fixtures}}/echo-agent/agent.toml {{dev_project}}/echo-agent/
    cp {{fixtures}}/upper-agent/agent.wasm {{dev_project}}/upper-agent/
    cp {{fixtures}}/upper-agent/agent.toml {{dev_project}}/upper-agent/
    cp {{fixtures}}/pensieve/agent.wasm {{dev_project}}/pensieve/
    cp {{fixtures}}/pensieve/agent.toml {{dev_project}}/pensieve/

# Build and deploy pensieve agent only
build-pensieve:
    cd agents/pensieve && cargo build --target wasm32-wasip1 --release
    # Copy to source dir (for running from agents/pensieve/)
    cp agents/pensieve/target/wasm32-wasip1/release/pensieve.wasm agents/pensieve/agent.wasm
    # Deploy to fixtures and dev-project
    mkdir -p {{fixtures}}/pensieve/mnt
    mkdir -p {{dev_project}}/pensieve/mnt
    cp agents/pensieve/agent.wasm {{fixtures}}/pensieve/agent.wasm
    cp agents/pensieve/agent.toml {{fixtures}}/pensieve/
    cp agents/pensieve/agent.wasm {{dev_project}}/pensieve/
    cp agents/pensieve/agent.toml {{dev_project}}/pensieve/

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
