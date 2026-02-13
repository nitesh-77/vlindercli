# ADR 082: Integration Test Experience

## Status

Accepted

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
  ├── check prerequisites (podman, nats-server, ollama)
  ├── verify Ollama endpoint responds
  ├── verify required models are pulled (tests/required-models.txt)
  ├── warn if VLINDER_OPENROUTER_API_KEY is absent
  ├── check container images (echo-container)
  ├── clean NATS JetStream state and start fresh
  ├── create date-stamped run directory
  └── VLINDER_INTEGRATION_RUN=<path> cargo test --test '*' -- --ignored --test-threads=1
```

The just recipe runs once, before any tests. It either succeeds (all prerequisites met) or fails with a clear message telling the developer what to install or start. No Rust code checks prerequisites — if you're running, you can assume the infrastructure is there.

### Date-stamped run directories

Each `just run-integration-tests` invocation creates a date-stamped directory:

```
/tmp/vlinder-integration/
├── 2026-02-13-143052/                    # this run
│   ├── checkout_shows_trailers_and_state/
│   │   └── .vlinder/
│   │       └── conversations/            # real git repo, inspectable
│   ├── promote_moves_main_and_labels_old/
│   │   └── .vlinder/
│   └── ...
├── 2026-02-12-091500/                    # yesterday's run, still inspectable
│   └── ...
```

The just recipe creates the run directory and passes its path to tests via `VLINDER_INTEGRATION_RUN`. Each test creates its own subdirectory named after itself, with a `.vlinder` inside. This is the test's `VLINDER_DIR`.

Directories are never cleaned up by the test runner. Previous runs accumulate and can be inspected at any time. `/tmp` cleans itself on reboot. Developers can `rm -rf /tmp/vlinder-integration/` manually if they want.

### Isolated VLINDER_DIR per test

Tests that produce artifacts (conversations, DBs) create their own `.vlinder` directory under the run directory and set `VLINDER_DIR` to point to it. The `vlinder_dir()` function in `src/config.rs` already respects this env var. The shared helper lives in `tests/common/mod.rs`:

```rust
pub fn test_vlinder_dir(test_name: &str) -> PathBuf {
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

Stateless tests (NATS connect, Ollama catalog, OpenRouter catalog) don't need VLINDER_DIR isolation — they just hit an endpoint and check the response. They still benefit from the just recipe's prerequisite checking.

### NATS JetStream clean start

NATS JetStream stores state (streams, consumers, messages) in a working directory. Leftover state from previous runs causes phantom messages and stale consumers — an entire class of flaky failures.

The just recipe stops any previously started NATS (via pid file), cleans the storage directory, and starts fresh:

```bash
rm -rf /tmp/vlinder-nats
nats-server -js -sd /tmp/vlinder-nats -p 4222 --pid /tmp/vlinder-nats/nats.pid &
```

A cleanup trap stops NATS when the recipe exits (success or failure).

### Optional prerequisites: graceful skip

Some prerequisites are optional. The OpenRouter tests require `VLINDER_OPENROUTER_API_KEY`, which most developers won't have. These are handled with a two-layer approach:

1. **Just recipe**: warns upfront that OpenRouter tests will be skipped
2. **Rust tests**: check for the key themselves and `return` early with an `eprintln!` skip message

This means OpenRouter tests report as "ok" (not "failed") when the key is absent. The developer sees the warning before tests start and the skip messages in test output. If they set the key, everything runs automatically.

```rust
fn openrouter_key_or_skip() -> Option<String> {
    match std::env::var("VLINDER_OPENROUTER_API_KEY") {
        Ok(key) if !key.is_empty() => Some(key),
        _ => {
            eprintln!("VLINDER_OPENROUTER_API_KEY not set — skipping");
            None
        }
    }
}
```

### AI-assisted debugging prompt

A separate just recipe generates a diagnostic prompt from a test's directory:

```
just debug-integration-test /tmp/vlinder-integration/2026-02-13-143052/checkout_shows_trailers_and_state
```

This recipe reads conversation git history, logs, config, and database file sizes, then formats everything into plain text the developer can paste into their AI tool. It makes no assumptions about which tool — just stdout.

### Prerequisite checking in just

The `run-integration-tests` recipe checks, in order:

1. **Binaries**: `podman`, `nats-server`, `ollama` must be on `$PATH`
2. **Ollama endpoint**: `http://localhost:11434/api/tags` must respond
3. **Required models**: each model in `tests/required-models.txt` must appear in `ollama list` (currently: `phi3`, `nomic-embed-text`)
4. **OpenRouter API key**: `VLINDER_OPENROUTER_API_KEY` — warns if absent, does not fail
5. **Container images**: `podman image exists localhost/echo-container:latest`

If any hard prerequisite fails, the recipe prints exactly what's missing and how to fix it, then exits. No partial runs.

### Test inventory

All `#[ignore]` tests run under `just run-integration-tests`:

| File | Tests | Prerequisites |
|---|---|---|
| `nats_integration_tests.rs` | `connect_to_localhost` | NATS |
| `container_runtime_tests.rs` | `container_runtime_executes_echo_agent` | Podman, echo-container image |
| `ollama_integration_tests.rs` | `lists_models_from_ollama`, `resolves_model_from_ollama`, `embeds_with_ollama_server`, `infers_with_ollama_server` | Ollama, phi3, nomic-embed-text |
| `openrouter_integration_tests.rs` | `lists_models_from_openrouter`, `resolves_model_from_openrouter`, `infers_with_openrouter_api` | `VLINDER_OPENROUTER_API_KEY` (optional) |
| `loader_integration_tests.rs` | `load_agent_with_file_uri`, `load_fleet_with_file_uri` | `tests/fixtures/` (in repo) |
| `timeline_integration_tests.rs` | `checkout_shows_trailers_and_state`, `promote_moves_main_and_labels_old`, `fork_creates_independent_branch`, `checkout_then_promote_full_workflow` | Git (uses `GitDagWorker` directly) |

### Shared test helpers: `tests/common/mod.rs`

Integration test binaries that need shared setup include `mod common;`. The module provides:

- `test_vlinder_dir(name)` — creates isolated VLINDER_DIR, sets env var
- `test_conversations_worker(vlinder_dir)` — opens a `GitDagWorker` at the conversations subdirectory
- `conversations_path(vlinder_dir)` — resolves the conversations directory path
- `git(dir, args)` — runs a git command and returns stdout
- `read_trailer(dir, commit, key)` — reads a git trailer value from a commit
- `read_head_sha(dir)` — gets the current HEAD SHA
- `make_invoke(...)` / `make_complete(...)` — factory functions for test messages

### What is NOT in scope

- CI integration (future ADR — needs Docker-in-Docker or similar)
- Performance benchmarks (different concern, different infrastructure)
- Mocking external services (defeats the purpose of integration tests)
- Full daemon-based end-to-end tests (deploy → invoke → verify conversation) — requires daemon lifecycle management, separate iteration

## Consequences

- `just run-integration-tests` is the single entry point — no guessing which tests to run or what to start first
- Prerequisite failures produce actionable messages, not cryptic Rust panics
- Optional prerequisites (OpenRouter) skip gracefully instead of failing the run
- Tests that produce artifacts get their own `VLINDER_DIR` — no collisions, no `~/.vlinder` corruption
- `--test-threads=1` eliminates env var race conditions
- Date-stamped run directories accumulate — every run is inspectable, nothing is lost
- Every stateful test prints its `VLINDER_DIR` path on startup — when it fails, the path is right there
- `just debug-integration-test <path>` generates a diagnostic prompt for AI-assisted debugging
- The `#[ignore]` convention is preserved — `cargo test` still runs the fast unit tests by default
- All 15 existing integration tests are migrated to this infrastructure
