# Vlinder directory (later: ~/.vlinder)
vlinder_dir := ".vlinder"

# Build all wasm agents and copy to .vlinder/agents/{name}/
build-agents:
    mkdir -p {{vlinder_dir}}/agents/echo-agent
    mkdir -p {{vlinder_dir}}/agents/upper-agent
    mkdir -p {{vlinder_dir}}/agents/pensieve
    cd agents/echo-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/upper-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/pensieve && cargo build --target wasm32-wasip1 --release
    cp agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm {{vlinder_dir}}/agents/echo-agent/
    cp agents/echo-agent/Vlinderfile {{vlinder_dir}}/agents/echo-agent/
    cp agents/upper-agent/target/wasm32-unknown-unknown/release/upper_agent.wasm {{vlinder_dir}}/agents/upper-agent/
    cp agents/upper-agent/Vlinderfile {{vlinder_dir}}/agents/upper-agent/
    cp agents/pensieve/target/wasm32-wasip1/release/pensieve.wasm {{vlinder_dir}}/agents/pensieve/
    cp agents/pensieve/Vlinderfile {{vlinder_dir}}/agents/pensieve/

# Build and deploy pensieve agent only
build-pensieve:
    mkdir -p {{vlinder_dir}}/agents/pensieve
    cd agents/pensieve && cargo build --target wasm32-wasip1 --release
    cp agents/pensieve/target/wasm32-wasip1/release/pensieve.wasm {{vlinder_dir}}/agents/pensieve/
    cp agents/pensieve/Vlinderfile {{vlinder_dir}}/agents/pensieve/

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
    rm -rf {{vlinder_dir}}/agents

# Check license compliance (fails on GPL/copyleft)
license-check:
    cargo deny check licenses
