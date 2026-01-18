# Build all wasm agents and copy to agent_wasm/
build-agents:
    mkdir -p agent_wasm
    cd agents/echo-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/upper-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/reader-agent && cargo build --target wasm32-unknown-unknown --release
    cp agents/echo-agent/target/wasm32-unknown-unknown/release/echo_agent.wasm agent_wasm/
    cp agents/upper-agent/target/wasm32-unknown-unknown/release/upper_agent.wasm agent_wasm/
    cp agents/reader-agent/target/wasm32-unknown-unknown/release/reader_agent.wasm agent_wasm/

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
    rm -rf agent_wasm
