# Vlinder directory (later: ~/.vlinder)
vlinder_dir := ".vlinder"

# Build all wasm agents and copy to .vlinder/agents/{name}/
build-agents:
    mkdir -p {{vlinder_dir}}/agents/echo-agent
    mkdir -p {{vlinder_dir}}/agents/upper-agent
    mkdir -p {{vlinder_dir}}/agents/reader-agent
    cd agents/echo-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/upper-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/reader-agent && cargo build --target wasm32-unknown-unknown --release
    cp agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm {{vlinder_dir}}/agents/echo-agent/
    cp agents/upper-agent/target/wasm32-unknown-unknown/release/upper_agent.wasm {{vlinder_dir}}/agents/upper-agent/
    cp agents/reader-agent/target/wasm32-unknown-unknown/release/reader_agent.wasm {{vlinder_dir}}/agents/reader-agent/

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
