//! Agent container health observation.
//!
//! Captures health check results as HealthSnapshots in a sliding window.
//! The sidecar uses this during startup (wait_for_agent) and can poll
//! between invocations.

use std::time::{Duration, Instant};

use vlinder_core::domain::{HealthSnapshot, HealthWindow};

/// Check the agent's health endpoint once and record the observation.
pub fn check_once(client: &ureq::Agent, url: &str, health: &mut HealthWindow) -> Option<u16> {
    let check_start = Instant::now();
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let status_code = match client.get(url).call() {
        Ok(_) => 200,
        Err(ureq::Error::Status(code, _)) => code,
        Err(_) => 0,
    };

    health.push(HealthSnapshot {
        timestamp_ms,
        latency_ms: check_start.elapsed().as_millis() as u64,
        status_code,
    });

    if status_code == 200 {
        Some(status_code)
    } else {
        None
    }
}

/// Poll the agent's health endpoint until it returns 200 or the deadline passes.
///
/// Every poll attempt is recorded as a HealthSnapshot — including
/// failures before the container is ready.
pub fn wait_for_ready(
    client: &ureq::Agent,
    health: &mut HealthWindow,
    port: u16,
    agent_name: &str,
) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{}/health", port);
    let deadline = Instant::now() + Duration::from_secs(60);

    tracing::info!(
        event = "sidecar.waiting",
        agent = %agent_name,
        port = port,
        "Waiting for agent container to become ready"
    );

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "agent container did not become ready within 60 seconds (port {})",
                port
            ));
        }

        if check_once(client, &url, health).is_some() {
            tracing::info!(
                event = "sidecar.agent_ready",
                agent = %agent_name,
                "Agent container is ready"
            );
            return Ok(());
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}
