//! Integration tests for Daemon that require agent fixtures.

use std::path::PathBuf;

use vlindercli::domain::{Daemon, Harness};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

fn reverse_agent_toml() -> String {
    let wasm_path = fixture_path("agents/reverse-agent/reverse-agent.wasm");
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

    // Deploy agent (runtime discovers automatically)
    let agent_id = daemon.harness.deploy(&reverse_agent_toml()).unwrap();

    // Invoke via harness
    let job_id = daemon.harness.invoke(&agent_id, "hello").unwrap();

    // Tick until complete
    let result = loop {
        daemon.tick();
        if let Some(result) = daemon.harness.poll(&job_id) {
            break result;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    };

    assert_eq!(result, "olleh");
}
