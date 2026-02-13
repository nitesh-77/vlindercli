# ADR 082: Integration Test Experience

## Status

Draft

## Context

The current integration tests are fragile and thin. `nats_integration_tests.rs` checks if `connect()` works. `container_runtime_tests.rs` runs a single echo invoke. Each test is `#[ignore]` with no shared infrastructure. There is no test that exercises a real workflow end-to-end.

Worse, every test reimplements its own prerequisite checking: "is NATS up?", "is Podman installed?", "is the echo container built?". This logic is scattered across Rust `#[ignore]` annotations and implicit assumptions. When a test fails, the developer has to figure out whether it's a real failure or a missing prerequisite.

Integration tests touch the real `~/.vlinder` directory. Parallel test runs collide. Leftover state from previous runs causes false failures.

Meanwhile, developers increasingly use AI coding tools (Claude Code, Gemini, Cursor, etc.) to contribute. A failed integration test produces a directory full of logs, conversation repos, and config — exactly the context an AI tool needs to diagnose the problem. But today that context is scattered or cleaned up.

## Decision

### Two layers: just handles infrastructure, Rust handles behavior

**`just`** owns prerequisites and infrastructure. **Rust** owns test behavior.

```
just run-integration-tests
  ├── check prerequisites (ollama, nats, podman)
  ├── verify endpoints respond
  ├── verify required models are pulled
  ├── clean NATS JetStream state
  ├── create date-stamped run directory
  └── VLINDER_INTEGRATION_RUN=<path> cargo test --test '*' -- --ignored --test-threads=1
```

The just recipe runs once, before any tests. It either succeeds (all prerequisites met) or fails with a clear message telling the developer what to install or start. No Rust code checks prerequisites — if you're running, you can assume the infrastructure is there.

### Date-stamped run directories

Each `just run-integration-tests` invocation creates a date-stamped directory:

```
/tmp/vlinder-integration/
├── 2026-02-13-143052/                    # this run
│   ├── test_checkout_navigates/
│   │   └── .vlinder/
│   │       ├── conversations/
│   │       ├── logs/
│   │       ├── config.toml
│   │       └── *.db
│   ├── test_promote_moves_main/
│   │   └── .vlinder/
│   └── ...
├── 2026-02-12-091500/                    # yesterday's run, still inspectable
│   └── ...
```

The just recipe creates the run directory and passes its path to tests via `VLINDER_INTEGRATION_RUN`. Each test creates its own subdirectory named after itself, with a `.vlinder` inside. This is the test's `VLINDER_DIR`.

Directories are never cleaned up by the test runner. Previous runs accumulate and can be inspected at any time. `/tmp` cleans itself on reboot. Developers can `rm -rf /tmp/vlinder-integration/` manually if they want.

### Isolated VLINDER_DIR per test

Every integration test creates its own `.vlinder` directory under the run directory and sets `VLINDER_DIR` to point to it. The `vlinder_dir()` function already respects this env var.

```rust
fn test_vlinder_dir(test_name: &str) -> PathBuf {
    let run_dir = std::env::var("VLINDER_INTEGRATION_RUN")
        .expect("VLINDER_INTEGRATION_RUN not set — use `just run-integration-tests`");
    let test_dir = PathBuf::from(run_dir).join(test_name).join(".vlinder");
    std::fs::create_dir_all(&test_dir).unwrap();
    std::env::set_var("VLINDER_DIR", &test_dir);
    eprintln!("VLINDER_DIR={}", test_dir.display());
    test_dir
}
```

`std::env::set_var` is process-global and not thread-safe. Tests **must** run sequentially: `--test-threads=1`. This is enforced by the just recipe. Integration tests are slow (real services, real containers) so parallelism within a process buys nothing and causes coordination headaches.

The `eprintln!` at the start of every test prints the `VLINDER_DIR` path. When a test fails, the path is visible in the output — no hunting.

### NATS JetStream clean start

NATS JetStream stores state (streams, consumers, messages) in a working directory. Leftover state from previous runs causes phantom messages and stale consumers — an entire class of flaky failures.

The just recipe starts NATS with a known storage directory (`/tmp/vlinder-nats/`) and cleans it before each run:

```bash
rm -rf /tmp/vlinder-nats
nats-server -js -sd /tmp/vlinder-nats &
```

If the developer already has NATS running, the recipe detects this and prompts them to restart with the expected storage path.

### AI-assisted debugging prompt

A separate just recipe generates a diagnostic prompt from a test's directory:

```
just debug-integration-test /tmp/vlinder-integration/2026-02-13-143052/test_checkout_navigates
```

This recipe:

1. Reads `.vlinder/logs/` — the tracing output
2. Reads `.vlinder/conversations/` — `git log --oneline` if it exists
3. Reads `.vlinder/config.toml` — the test configuration
4. Lists `.vlinder/*.db` files and their sizes
5. Formats everything into a diagnostic prompt the developer can paste into their AI tool

The recipe outputs plain text to stdout. It makes no assumptions about which AI tool the developer uses. Developers who don't use AI tools never see this — it's a separate recipe, not part of the test output. Developers who do get a fast path from failure to diagnosis.

### Prerequisite checking in just

The `run-integration-tests` recipe checks:

1. **Podman**: `podman --version` succeeds
2. **NATS**: `nats-server --version` succeeds, endpoint `nats://localhost:4222` is reachable
3. **Ollama**: `ollama --version` succeeds, endpoint `http://localhost:11434` responds
4. **Models**: `ollama list` includes required models (documented in `tests/required-models.txt`)
5. **Container images**: `podman image exists localhost/echo-container:latest` (fails with: "Run `just build-echo-container` first")

If any check fails, the recipe prints exactly what's missing and how to fix it, then exits. No partial runs. No building containers silently — the developer should know what they're building and why.

### Test structure

Integration test files in `tests/` follow a convention:

- Each file tests one workflow or subsystem end-to-end
- All tests in the file are `#[ignore]` (run only via `just run-integration-tests`)
- A shared helper module in `tests/` provides `test_vlinder_dir()` and common setup
- Tests are named descriptively: `conversation_produces_git_commits_with_trailers`, not `test_1`

### What is NOT in scope

- CI integration (future ADR — needs Docker-in-Docker or similar)
- Performance benchmarks (different concern, different infrastructure)
- Mocking external services (defeats the purpose of integration tests)

## Consequences

- `just run-integration-tests` is the single entry point — no guessing which tests to run or what to start first
- Prerequisite failures produce actionable messages, not cryptic Rust panics
- Tests cannot collide or corrupt each other — each gets its own `VLINDER_DIR`
- `--test-threads=1` eliminates env var race conditions
- Date-stamped run directories accumulate — every run is inspectable, nothing is lost
- Every test prints its `VLINDER_DIR` path on startup — when it fails, the path is right there
- `just debug-integration-test <path>` generates a diagnostic prompt for AI-assisted debugging
- Developers contributing with AI tools get a better loop: fail → see path → generate prompt → fix
- The `#[ignore]` convention is preserved — `cargo test` still runs the fast unit tests by default
