//! Integration tests for Daemon that require agent fixtures.

use std::path::PathBuf;

use vlindercli::domain::Daemon;

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

fn reverse_agent_toml() -> String {
    let wasm_path = fixture_path("agents/reverse-agent/agent.wasm");
    format!(
        r#"
        name = "reverse-agent"
        description = "Reverses input"
        id = "file://{}"
        [requirements]
        services = []
        "#,
        wasm_path.display()
    )
}

#[test]
fn daemon_invokes_agent_and_returns_result() {
    let mut daemon = Daemon::new();

    // Invoke via daemon (which delegates to harness)
    let job_id = daemon.invoke(&reverse_agent_toml(), "hello").unwrap();

    // Tick until complete
    let result = loop {
        daemon.tick();
        if let Some(result) = daemon.poll(&job_id) {
            break result;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    };

    assert_eq!(result, "olleh");
}
