# Build all wasm agents
build-agents:
    cd agents/echo-agent && cargo build --target wasm32-unknown-unknown --release
    cd agents/upper-agent && cargo build --target wasm32-unknown-unknown --release

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
